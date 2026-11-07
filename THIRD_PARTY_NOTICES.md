# 第三方说明

本项目自己的代码采用根目录 MIT 许可；依赖和工具继续适用各自许可证，不因本项目许可而改变。

- Rust 依赖版本由 `campus-tray/Cargo.lock` 锁定。主要直接依赖包括 base64、chrono、encoding_rs、mime、serde、serde_json、reqwest、winreg、windows 和 windows-sys。具体传递依赖及许可请查看对应 crate 元数据和上游发布内容；此处不将所有第三方依赖统一声明为 MIT。
- Android Gradle Wrapper 的上游许可及 NOTICE 原样保留在 `shunet-android/gradle/wrapper/LICENSE` 和 `NOTICE.txt`。Wrapper JAR 是构建工具，不是应用业务代码。
- Android SDK、JDK、Gradle 和 Android Gradle Plugin 的使用及分发受各自许可约束，仓库不分发这些 SDK 或工具安装包。
- Windows 系统 API 与 Android 系统 API 的实现由操作系统提供。
- ipwho.is 和 Cloudflare 测速为外部网络服务，其可用性、服务条款和数据处理方式由服务提供方决定。

校园服务名称和网址用于导航，不表示相关学校、运营商或服务方为本项目提供背书。
