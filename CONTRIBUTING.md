# 参与开发

请先提交 Issue 描述使用场景或可复现问题。两端独立版本号，修改某一平台不必提升另一平台版本。

Windows 逻辑位于 `campus-tray/src`，原生资源位于 `campus-tray/assets`；Android 位于 `shunet-android/app/src/main`。保持网络请求在后台执行，保留认证前的当前 Wi-Fi 身份核对、明确离线证据及单次凭据提交约束。

Windows 使用 `scripts/build-windows.ps1` 运行默认测试并构建；Android 使用 Gradle Wrapper 执行 `:app:assembleDebug :app:lintDebug`。`ignored` 测试可能访问实网、打开原生界面或注册计划任务，仅在了解具体测试行为时单独选择运行。

PR 请说明具体行为变化、验证环境和已知限制。只提交源码及必要资源，不提交本机设置、日志、门户快照、SDK 路径、签名密钥或编译产物。校园实机认证未验证时请如实注明。

提交前请在本仓库配置 GitHub 提供的 `users.noreply.github.com` 隐私邮箱，避免将个人邮箱写入公开的提交或标签记录。Windows 发布请使用构建脚本提供的路径重映射；自行运行 Cargo 时也要去掉二进制中的本机用户目录和工作区路径。
