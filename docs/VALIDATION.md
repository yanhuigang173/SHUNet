# 首次发布验证

整理日期：2026-11-07。两端源码沿用当前最新版，仓库整理未提升软件版本号。

## Windows v0.14.3

- Rust：`rustc 1.99.0`，目标 `x86_64-pc-windows-gnu`，MinGW-w64。
- 对整理后的源码执行 `cargo test --locked --offline`：**85 passed，0 failed，16 ignored**。
- 默认忽略了实网认证、原生界面和计划任务等集成测试，本次未执行这些测试。
- 对整理后的源码执行锁定依赖、启用路径重映射的 release 构建，成功生成 x64 EXE；PE 版本资源的 ProductVersion / FileVersion 均为 `0.14.3`。文件大小与摘要以当前 Release 附件为准。
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
- 本地 Markdown 链接检查通过；检查常见私钥、GitHub 令牌以及本机路径，公开的 Git 提交和标签使用 GitHub 隐私邮箱。
- 按维护者要求关闭 GitHub Actions 自动构建，停止运行中的任务并移除活动工作流。首轮远端验证曾在工具准备阶段失败；本次发布以本文列出的本机验证结果为准，不宣称远端 CI 已通过。

## 发布隐私复核

- 在本机内存中将本应用已保存的账号、解密密码及加密密码数据作为精确比对参考，不输出、不上传参考值。
- 对所有可达源码历史、提交和标签元数据，以及发布附件和压缩包内部文件，检查 UTF-8 / UTF-16、JSON 转义和 URL 编码形式；未发现已保存校园账号或密码。
- Git 作者和标签身份统一为项目公开 GitHub 账号及其 `users.noreply.github.com` 隐私邮箱。
- Windows EXE 使用路径重映射重新构建，去掉本机用户名和工作区路径；同版本便携 ZIP 与 SHA-256 清单同步替换。默认测试再次为 85 passed、0 failed、16 ignored。
- Android APK 结构、签名及摘要保持不变，未发现本机账号、密码或个人编译目录。
- 本次发布产生的失败 / 取消构建记录和日志已清理，不保留活动的自动构建工作流。

检查范围是本仓库当前可达内容及当前下载附件，不代表能够收回他人已下载的副本或直接清除 GitHub 服务端的历史缓存。
