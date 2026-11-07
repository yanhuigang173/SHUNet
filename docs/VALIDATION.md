# 首次发布验证

整理日期：2026-11-07。两端源码沿用当前最新版，仓库整理未提升软件版本号。

## Windows v0.14.3

- Rust：`rustc 1.99.0`，目标 `x86_64-pc-windows-gnu`，MinGW-w64。
- 对整理后的源码执行 `cargo test --locked --offline`：**85 passed，0 failed，16 ignored**。
- 默认忽略了实网认证、原生界面和计划任务等集成测试，本次未执行这些测试。
- 对整理后的源码执行锁定依赖的 release 构建，成功生成 x64 EXE；PE 版本资源的 ProductVersion / FileVersion 均为 `0.14.3`，文件大小 2,875,392 字节。
- 检查导入表，依赖项为 Windows 系统 DLL，未发现额外的 MinGW 运行时 DLL 导入。
- 生成单文件 EXE 和便携 ZIP，并计算 SHA-256。实际下载文件以 Release 附件校验清单为准。

首次受限环境中的测试不能访问回环测试服务器及部分临时文件；在正常本机环境重跑后，默认测试全部通过。本次未对真实校园会话执行登录、下线、重连或电源操作。

## Android v0.4.0

- 发布现有 `SHUNet-Android-v0.4.0-debug.apk`，未改写或重新签名，大小 62,273 字节。
- ZIP CRC 检查通过；读取二进制 AndroidManifest，确认包名 `com.shunet.android`、versionName `0.4.0`、versionCode `4`、minSdk `26`、targetSdk `36`。
- 使用 Android 官方 `com.android.tools.build:apksig:8.13.2` 的 `ApkVerifier` 验证通过，APK Signature Scheme v2 有效。
- 签名主体为 `C=US, O=Android, CN=Android Debug`，Manifest 中 `debuggable=true`，因此明确作为调试签名预览包发布。
- 声明的运行权限为 `INTERNET` 与 `ACCESS_NETWORK_STATE`，Manifest 设置 `allowBackup=false`。
- 签名证书 SHA-256：`f03503900ed7e3ceb4777a715ff3d60fb9b1c2ffeb3b24fd086e14071371cb60`。
- APK SHA-256：`6a5cba3d653aa22da09db070ba04d96baf0129b5b5932d312288f48be68021a5`。

尝试对整理后的 Android 源码离线构建时，本机原 SDK 缓存缺少 `platforms/android-36/android.jar`，构建脚本在编译前停止。因此本次仅完成现有 APK 的结构、元数据和签名核验，没有完成 Android 源码重建或 Lint，也没有进行 Android 校园 Wi-Fi 实机认证验证。APK 校验不能证明网络登录一定成功。

## 仓库检查

- 只收入 Windows 托盘版和 Android 版最新源码，未收入早期桌面版。
- 未收入历史二进制、基准测试快照、门户抓取、本机配置、日志、SDK 路径或签名私钥。
- 本地 Markdown 链接检查通过；已检查常见私钥、GitHub 令牌和本机用户绝对路径模式，无命中。
- 按维护者要求关闭 GitHub Actions 自动构建，停止运行中的任务并移除活动工作流。首轮远端验证曾在工具准备阶段失败；本次发布以本文列出的本机验证结果为准，不宣称远端 CI 已通过。
