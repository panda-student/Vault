vault 0.5.8 · Windows x64

安装版：vault-0.5.8-setup.exe，默认 D:\vault，可在向导中改目录。重装沿用原目录，卸载保留 data。
便携版：vault-0.5.8.zip，解压后启动 vault.exe。需要系统已有 Microsoft Edge WebView2 Runtime，不捆绑 Node.js、Python、Rust 或数据库驱动。

支持账号密码、API 凭据、数据库信息、自由标签、垃圾桶与本地 MCP 引用填写。MCP 服务和配置键均为 vault。AI 接口不返回密码、API Key 或 Token 原文；程序在本地解密并写入用户指定目标。目标文件或网页可能被其他工具读取。

覆盖安装前请保存界面修改。安装器等待正在执行的 MCP 操作完成，自动退出旧桌面、替换程序、按原数据目录重启，并恢复原 MCP 连接，无需退出 Trae。新版启动失败时自动恢复旧程序。0.5.3～0.5.6 使用此安装包升级一次以切换到小写 vault.exe；0.5.2 及以前的旧直连 MCP 首次迁移仍需重连一次。此后可继续在设置中使用签名 ZIP 在线更新。

数据默认在程序旁 data；自定义数据目录会保留。首次启动为空库。内容使用 AES-256-GCM 加密，数据密钥受当前 Windows 用户和电脑的 DPAPI 保护；便携不代表可以跨电脑解密。备份时退出程序并断开 MCP，复制完整 data，包含可能存在的 WAL 文件。

项目采用 Apache-2.0，第三方组件保留各自协议。请保留 LICENSE.txt、NOTICE.txt、THIRD_PARTY_NOTICES.txt 和 third-party-sources。
项目及下载：https://github.com/panda-student/vault
