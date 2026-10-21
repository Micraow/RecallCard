//! 界面服务的唯一不变量：没有访问令牌，读不到任何资料。
//! 只起临时库与临时前端目录，不碰真实数据。
use recallcard::{
    http::{self, UiOptions},
    vault::Vault,
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    thread,
};
use tempfile::{tempdir, TempDir};

mod support;

fn started() -> (TempDir, u16, String) {
    let dir = tempdir().unwrap();
    let vault = dir.path().join("资料库");
    support::isolate_state();
    Vault::init(&vault).unwrap();
    let web = dir.path().join("web");
    fs::create_dir_all(&web).unwrap();
    fs::write(
        web.join("index.html"),
        "<!doctype html><title>合成界面</title>",
    )
    .unwrap();
    let bound = http::bind(UiOptions {
        vault,
        scope: "personal".into(),
        web_dir: web,
        port: 0,
        open_browser: false,
    })
    .unwrap();
    let port = bound.port();
    let token = bound.token().to_owned();
    thread::spawn(move || {
        let _ = http::run(bound);
    });
    (dir, port, token)
}

/// 裸 HTTP/1.1 请求：不经过任何客户端库，失败时能看清是连接问题还是状态码问题。
fn status(port: u16, path: &str, cookie: Option<&str>) -> u16 {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut request =
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
    if let Some(cookie) = cookie {
        request.push_str(&format!("Cookie: {cookie}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
        .split_whitespace()
        .nth(1)
        .unwrap_or("没有响应")
        .parse()
        .unwrap_or(0)
}

#[test]
fn local_ui_serves_nothing_without_the_startup_token() {
    let (_dir, port, token) = started();

    assert_eq!(status(port, "/", None), 401, "首页不该在没有令牌时出现");
    assert_eq!(status(port, "/api/state", None), 401, "状态接口不该免令牌");
    assert_eq!(
        status(port, "/api/state", Some("rc_token=wrong-token")),
        401,
        "错误令牌不该放行"
    );

    let cookie = format!("rc_token={token}");
    assert_eq!(status(port, "/api/state", Some(&cookie)), 200);
    assert_eq!(status(port, &format!("/?token={token}"), None), 200);
}
