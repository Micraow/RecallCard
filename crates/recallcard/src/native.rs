//! 浏览器 Native Messaging：只读，范围与扩展 ID 来自本机配置。
use crate::{context::Context, model::Result, policy::Access, transport::invoke, Vault};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::Path,
};
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NativeRequest {
    protocol: String,
    request_id: String,
    nonce: String,
    action: String,
    #[serde(default)]
    arguments: Option<Value>,
    session_ref: String,
}
pub fn extension_origin(id: &str) -> Result<String> {
    if id.len() != 32 || !id.bytes().all(|b| (b'a'..=b'p').contains(&b)) {
        return Err("Chrome 扩展 ID 必须为 a–p 的 32 位字符串".into());
    }
    Ok(format!("chrome-extension://{id}/"))
}
pub fn serve_native(vault: &Vault, access: Access, extension: &str, origin: &str) -> Result<()> {
    serve_native_io(
        vault,
        access,
        extension,
        origin,
        std::io::stdin().lock(),
        std::io::stdout().lock(),
    )
}
pub fn serve_native_io<R: Read, W: Write>(
    vault: &Vault,
    access: Access,
    extension: &str,
    origin: &str,
    mut reader: R,
    mut writer: W,
) -> Result<()> {
    if origin != extension_origin(extension)? {
        return Err("Native Messaging 来源扩展不在允许名单".into());
    }
    let context = Context::new(vault, access);
    let mut cache: BTreeMap<String, (String, Value)> = BTreeMap::new();
    loop {
        let mut prefix = [0u8; 4];
        match reader.read(&mut prefix[..1]) {
            Ok(0) => return Ok(()),
            Ok(_) => {}
            Err(e) => return Err(e.to_string()),
        }
        reader
            .read_exact(&mut prefix[1..])
            .map_err(|_| "Native 消息头不完整")?;
        let length = u32::from_ne_bytes(prefix) as usize;
        if length == 0 || length > 256 * 1024 {
            return Err("Native 输入长度超出 1–262144 字节范围".into());
        }
        let mut bytes = vec![0u8; length];
        reader
            .read_exact(&mut bytes)
            .map_err(|_| "Native 消息正文不完整")?;
        let response = match serde_json::from_slice::<NativeRequest>(&bytes) {
            Err(_) => json!({"ok":false,"error":"Native 请求 JSON/schema 无效"}),
            Ok(request) => {
                let valid = request.protocol == "recallcard.action/1"
                    && request.request_id.len() <= 128
                    && !request.request_id.is_empty()
                    && (16..=256).contains(&request.nonce.len())
                    && !request.session_ref.is_empty()
                    && request.session_ref.len() <= 2048;
                if !valid {
                    json!({"ok":false,"error":"协议、nonce、request_id 或会话绑定无效"})
                } else {
                    let key = format!("{}\0{}", request.session_ref, request.request_id);
                    let hash = crate::hash(&bytes);
                    if let Some((previous, response)) = cache.get(&key) {
                        if *previous == hash {
                            response.clone()
                        } else {
                            json!({"ok":false,"error":"同一 request_id 对应不同请求"})
                        }
                    } else if cache.len() >= 120 {
                        json!({"ok":false,"error":"本连接请求达到上限，请重新连接"})
                    } else {
                        let response = match invoke(
                            &context,
                            &request.action,
                            request.arguments.unwrap_or_else(|| json!({})),
                        ) {
                            Ok(result) => {
                                json!({"ok":true,"result":result,"request_id":request.request_id})
                            }
                            Err(error) => {
                                json!({"ok":false,"error":error,"request_id":request.request_id})
                            }
                        };
                        cache.insert(key, (hash, response.clone()));
                        response
                    }
                }
            }
        };
        let encoded = serde_json::to_vec(&response).map_err(|e| e.to_string())?;
        if encoded.len() > 1024 * 1024 {
            return Err("Native 响应超过浏览器 1 MiB 上限".into());
        }
        writer
            .write_all(&(encoded.len() as u32).to_ne_bytes())
            .map_err(|e| e.to_string())?;
        writer.write_all(&encoded).map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
    }
}
pub fn prepare_install(
    vault: &Vault,
    scopes: Vec<String>,
    extension: &str,
    output: &Path,
) -> Result<Value> {
    let origin = extension_origin(extension)?;
    let access = Access::new(scopes)?;
    if cfg!(windows) {
        return Err(
            "Windows 原生包装器尚未验收；请按浏览器文档配置受限启动包装器，不会自动修改注册表"
                .into(),
        );
    }
    crate::vault::reject_symlink(output)?;
    fs::create_dir_all(output).map_err(|e| e.to_string())?;
    let output = fs::canonicalize(output).map_err(|e| e.to_string())?;
    let script = output.join("recallcard-native-host");
    let manifest = output.join("com.recallcard.host.json");
    if script.exists() || manifest.exists() {
        return Err("安装目录已包含生成文件，请选择新的目录以保留原配置".into());
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    fn quote(s: &str) -> String {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
    let mut command = format!(
        "#!/bin/sh\nexec {} --vault {} native-host --allowed-extension {}",
        quote(&exe.to_string_lossy()),
        quote(&vault.root().to_string_lossy()),
        quote(extension)
    );
    for scope in access.scopes() {
        command.push_str(&format!(" --scope {}", quote(&scope)));
    }
    command.push_str(" \"$@\"\n");
    fs::write(&script, command).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    let value = json!({"name":"com.recallcard.host","description":"RecallCard 本机只读上下文桥","path":script,"type":"stdio","allowed_origins":[origin]});
    fs::write(
        &manifest,
        serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(
        json!({"ok":true,"manifest":manifest,"wrapper":script,"registered":false,"note":"只生成待检查文件；请按浏览器与操作系统文档手动注册，不会修改浏览器权限或系统配置"}),
    )
}
