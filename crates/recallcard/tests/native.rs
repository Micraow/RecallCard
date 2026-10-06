use recallcard::{native, policy::Access, Vault};
use serde_json::{json, Value};
const EXT: &str = "abcdefghijklmnopabcdefghijklmnop";
fn request(action: &str) -> Value {
    json!({"protocol":"recallcard.action/1","request_id":"r_demo","nonce":"a-long-synthetic-nonce","session_ref":"chatgpt:demo","action":action,"arguments":{}})
}
fn frame(v: &Value) -> Vec<u8> {
    let body = serde_json::to_vec(v).unwrap();
    let mut out = (body.len() as u32).to_ne_bytes().to_vec();
    out.extend(body);
    out
}
fn run(input: Vec<u8>, origin: &str) -> Result<Vec<Value>, String> {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let mut out = Vec::new();
    native::serve_native_io(
        &v,
        Access::new(vec!["personal".into()]).unwrap(),
        EXT,
        origin,
        std::io::Cursor::new(input),
        &mut out,
    )?;
    let mut at = 0;
    let mut result = vec![];
    while at < out.len() {
        let len = u32::from_ne_bytes(out[at..at + 4].try_into().unwrap()) as usize;
        at += 4;
        result.push(serde_json::from_slice(&out[at..at + len]).unwrap());
        at += len;
    }
    Ok(result)
}
#[test]
fn allowed_browser_origin_can_only_read() {
    let origin = native::extension_origin(EXT).unwrap();
    assert_eq!(
        run(frame(&request("bootstrap")), &origin).unwrap()[0]["ok"],
        true
    );
    assert_eq!(
        run(frame(&request("capture")), &origin).unwrap()[0]["ok"],
        false
    );
}
#[test]
fn foreign_browser_origin_and_invalid_id_are_rejected() {
    assert!(run(frame(&request("bootstrap")), "https://chatgpt.com/").is_err());
    assert!(native::extension_origin("../evil").is_err());
}
#[test]
fn native_messages_are_bounded_and_require_complete_frames() {
    let origin = native::extension_origin(EXT).unwrap();
    assert!(run((262145u32).to_ne_bytes().to_vec(), &origin).is_err());
    assert!(run(vec![1, 2], &origin).is_err());
    assert!(run(vec![5, 0, 0, 0, b'{'], &origin).is_err());
}
#[test]
fn unknown_fields_and_invalid_nonce_are_rejected() {
    let origin = native::extension_origin(EXT).unwrap();
    let mut r = request("bootstrap");
    r["shell"] = json!("false");
    assert_eq!(run(frame(&r), &origin).unwrap()[0]["ok"], false);
    r.as_object_mut().unwrap().remove("shell");
    r["nonce"] = json!("short");
    assert_eq!(run(frame(&r), &origin).unwrap()[0]["ok"], false);
}
#[test]
fn duplicate_id_with_changed_arguments_is_not_executed() {
    let origin = native::extension_origin(EXT).unwrap();
    let r = request("bootstrap");
    let mut next = r.clone();
    next["arguments"] = json!({"budget_tokens":512});
    let mut input = frame(&r);
    input.extend(frame(&r));
    input.extend(frame(&next));
    let responses = run(input, &origin).unwrap();
    assert_eq!(responses[0], responses[1]);
    assert_eq!(responses[2]["ok"], false);
}
#[cfg(unix)]
#[test]
fn installer_generates_restricted_files_without_registering() {
    let d = tempfile::tempdir().unwrap();
    let v = Vault::init(&d.path().join("vault")).unwrap();
    let out = d.path().join("install");
    let result = native::prepare_install(&v, vec!["personal".into()], EXT, &out).unwrap();
    assert_eq!(result["registered"], false);
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(out.join("com.recallcard.host.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["allowed_origins"].as_array().unwrap().len(), 1);
    assert!(native::prepare_install(&v, vec!["personal".into()], EXT, &out).is_err());
}
