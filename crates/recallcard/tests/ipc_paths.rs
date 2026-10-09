//! 不创建 socket 的确定性路径检查；与实际 bind 权限验收分开。
#![cfg(unix)]
use recallcard::{
    ipc::{self, Client},
    policy::Access,
    Vault,
};
use std::{os::unix::ffi::OsStrExt, path::PathBuf, time::Duration};

#[test]
fn long_vault_path_changes_binding_but_not_default_endpoint_length() {
    let directory = tempfile::tempdir().unwrap();
    let short = Vault::init(&directory.path().join("short")).unwrap();
    let mut long_path = directory.path().join("long");
    for _ in 0..4 {
        long_path = long_path.join("x".repeat(80));
    }
    let long = Vault::init(&long_path).unwrap();
    let personal = Access::new(vec!["personal".into()]).unwrap();
    let work = Access::new(vec!["project:work".into()]).unwrap();
    let first = ipc::default_endpoint(&short, &personal).unwrap();
    let second = ipc::default_endpoint(&long, &personal).unwrap();
    let third = ipc::default_endpoint(&long, &work).unwrap();
    assert_eq!(first.parent(), second.parent());
    assert_eq!(second.parent(), third.parent());
    assert_eq!(
        first.as_os_str().as_bytes().len(),
        second.as_os_str().as_bytes().len()
    );
    assert_eq!(second.file_name().unwrap().as_bytes().len(), 29);
    assert_ne!(first, second);
    assert_ne!(second, third);
    assert!(!second.starts_with(long.root()));
}

#[test]
fn explicit_endpoint_byte_limit_is_checked_without_opening_a_socket() {
    let directory = tempfile::tempdir().unwrap();
    let vault = Vault::init(&directory.path().join("vault")).unwrap();
    let access = Access::new(vec!["personal".into()]).unwrap();
    let accepted = PathBuf::from(format!("/{}", "x".repeat(99)));
    let rejected = PathBuf::from(format!("/{}", "x".repeat(100)));
    assert!(Client::new(&vault, &access, accepted, Duration::from_secs(1)).is_ok());
    let error = Client::new(&vault, &access, rejected, Duration::from_secs(1))
        .err()
        .unwrap();
    assert!(error.contains("100 字节"));
    // Unix 的限制以 UTF-8 字节数而非字符数计。
    let unicode = PathBuf::from(format!("/{}", "界".repeat(34)));
    assert!(Client::new(&vault, &access, unicode, Duration::from_secs(1)).is_err());
}
