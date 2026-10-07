# 桌面依赖的补充许可证

部分 crate 发布包没有携带仓库根目录的许可证。打包时使用以下确切版本的上游原文，保留英文法律文本；不改写为中文。其余依赖直接使用 Cargo 包附带的许可证。

- alloc-stdlib 0.3.0：[上游 LICENSE](https://github.com/dropbox/rust-alloc-no-stdlib/blob/0a81fd6928ea3b33c8cd484aa4575d50ffb98012/LICENSE)
- defmt-parser 1.0.0：[上游 MIT](https://github.com/knurling-rs/defmt/blob/4a8cdb44891ed57b8ff5a023b6bec7137c48708f/LICENSE-MIT)，从其双许可选项中随包保留 MIT 文本
- dlopen2 0.8.2、dlopen2_derive 0.4.3：[共同仓库 LICENSE](https://github.com/OpenByteDev/dlopen2/blob/cc80e4a0a90d499b677fdf7743699b4b3a43a989/LICENSE)
- libappindicator-sys 0.9.0：取同一仓库提交 eafd1e3682a1247f595410266091e9684021cb6f 发布的 libappindicator 0.9.0 包内 LICENSE-MIT / LICENSE-APACHE；两个包的 `.cargo_vcs_info.json` 已核对一致
- tauri-plugin 2.7.1：取同一仓库提交 30da1fd6e17de6107ecc850c95dfb16b5729f2dd 发布的 tauri 2.12.1 包内 LICENSE-MIT / LICENSE-APACHE-2.0；两个包的 `.cargo_vcs_info.json` 已核对一致
- selectors 0.38.0：包内源文件声明 MPL-2.0，完整条款取 [Mozilla 官方 MPL 2.0](https://www.mozilla.org/en-US/MPL/2.0/)；页面可见正文转为纯文本，未改变条款内容

成品包对所有 MPL-2.0 依赖另行附带原样 Cargo 包源码，位于 `third-party-source/`。每项依赖的版本、原仓库、精确 crate 下载地址和随包许可证文件列在 `third-party-licenses/index.json`。构建未修改任何这些依赖的源文件，也没有把 WebKit/GTK 系统动态库打进成品包。
