# 安装、便携分发与在线更新

0.5.1 起，设置页支持检查、下载、取消和重启更新。启动和每小时自动检查，由用户确认重启。0.5.4 起改用签名 ZIP 更新；0.5.3 及以前版本需手动下载安装包或 ZIP 升级一次。

## 模块与权限

桌面 update_action → CoreSession.execute(Operation) → 核心权限服务 → UpdateService。五种更新操作均 DesktopOnly；MCP 工具发现和核心执行均不允许更新。更新服务管理版本、下载、验签和就绪状态；Windows 能力服务负责隐藏辅助进程、等待退出与重启。辅助进程使用当前 EXE 的临时副本，连接进程、共享核心及更新辅助进程均来自同一 EXE，不增加外部运行时。

桌面与 MCP 共享后台核心在打开数据库前按程序绝对路径获取共享文件锁。0.5.3 起，多个 stdio 连接进程通过当前 Windows 用户专用命名管道共用一份 MCP 核心（按 Windows 身份、EXE 绝对路径及规范化密码库目录隔离），不监听 TCP/HTTP 端口。共享核心按需启动，无客户端时空闲 5 秒退出。连接进程本身不打开数据库。

重启更新在核心操作屏障内发出暂停请求，等待所有连接已接收请求完成（最多 30 秒）；期间新请求明确返回 UPDATE_IN_PROGRESS，不排队、不重放写入。各连接关闭内部管道，共享核心释放存储与运行锁后，更新器才申请独占锁并替换程序。客户端的 stdio 进程与连接保留，更新成功或回退后重新连接唯一的新核心，内部恢复初始化握手。其他桌面窗口或尚未迁移的旧版 MCP 仍阻止替换；0.5.2 首次迁移需在 AI 客户端重连一次 MCP，无需退出客户端。替换完成后使用原 EXE 路径、原绝对 --data-dir 重启。界面阻止未保存凭据、暂存草稿、权限草稿和保存中的操作丢失。

## 签名发布协议

稳定源：https://github.com/panda-student/vault/releases/latest/download/latest.json

本项目使用签名信封格式（不是 Tauri 安装版更新格式）：

```json
{"payload":"base64(UTF-8 JSON)","signature":"base64(Ed25519 signature over exact payload bytes)"}
```

签名载荷 schema=2 包含 version、platform=windows-x86_64、固定仓库/版本目录的 vault-<version>.zip URL、程序 sha256/size、压缩包 archive_sha256/archive_size 和更新说明。先验签，只接受较新的正式版本。程序和 ZIP 各限 64 MiB；下载先校验 ZIP 长度和哈希，Rust ZIP 解析只读固定的 vault-<version>/vault.exe 成员，不解压任何其他路径，再校验程序长度、MZ 和哈希，安装前及辅助进程中再次校验。在线更新只替换程序；下载完整分发包后，用户也可手动更新 README、许可证和第三方声明。安装版与便携版使用同一套更新流程，不运行安装器。

公钥位于 src-tauri/update-public-key.txt；PKCS#8 DER 私钥的 Base64 文本保存为 GitHub Secret VAULT_UPDATE_SIGNING_KEY。本机备份在用户 .codex/secrets/local-password-manager/update-signing-key.dpapi，由当前用户 DPAPI 加密。该备份不能直接跨电脑解密，需要另行安排受保护的离线恢复备份；不要随意更换已发行客户端信任的公钥。更新验签与 Windows Authenticode/SmartScreen 签名互相独立。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-updater.ps1 -UploadSecret
```

上述命令初始化/恢复本机密钥并上传 Secret，不打印私钥。

## CI

现有 .github/workflows/ci-build.yml：

- main 推送：测试、构建，生成 preview-<commit> 预发行附件；不提供稳定更新描述。
- v* 标签：校验 package/Cargo/Tauri 版本，测试、构建、打包、签名并上传草稿 Release；附件齐全后发布为最新正式版。
- PR 和 ci-build：验证并保留 7 天 Actions 附件，不发布正式版。
- 不再将 EXE 提交回源码；使用标准 Windows runner。

正式发布需要修改三个版本文件及锁文件，再推送对应版本标签。Release 应用下载附件为 vault-<version>-setup.exe 和 vault-<version>.zip；另含 latest.json、SHA256SUMS、size.json、installer-size.json 元数据，不上传独立 vault.exe。Secret 缺失或不匹配时发布失败，不允许无签名降级。

## 数据与失败处理

更新不删除、移动或上传 data。数据库迁移仍由存储服务负责，沿用事务和升级前加密备份。旧程序保存在 .vault-update-<UUID>/previous.exe。新版须在 60 秒内完成核心、窗口与托盘初始化并回报令牌，否则终止本次启动的新版进程并尝试恢复旧 EXE。这不能证明全部业务功能正常。

EXE 回退不等于数据库回退。跨格式升级应保留迁移前备份，旧程序未必能读取未来格式。回退失败保留暂存目录供恢复。缓存位于 LocalAppData/LocalPasswordManager/updates，不进入发行包。

## 验证

Rust 测试覆盖签名、版本、哈希、跨进程锁和 MCP 固定禁止。原生验收使用两个独立构建、隔离数据目录、签名测试文件和实际窗口交互；报告放在忽略的 artifacts 验收目录。

仅 debug 构建接受 VAULT_UPDATE_TEST_SOURCE=http://127.0.0.1:<port>/latest.json；仍使用正式公钥验签。发行构建不接受更新地址或公钥的环境覆盖。

CI 迁移验收下载已发布的 v0.3.0 便携包并校验固定 SHA-256，覆盖真实旧版数据升级；本地仍可通过 VAULT_OLD_EXE 使用更早的 schema 1 版本。

命名管道设置显式受保护 DACL（当前 Windows 用户与 LocalSystem），拒绝远程连接；连接端校验管道服务端进程所属 Windows 身份。暂停请求只包含随机令牌与到期时间，不包含凭据、操作或客户端配置；失败路径按令牌解除暂停，辅助进程异常终止的遗留暂停请求 5 分钟后失效。

共享核心与多连接验收：`node scripts/verify-mcp-handoff.mjs`（可用 VAULT_EXE 指定程序）；更新/回退和连接保留验收：`node scripts/verify-update.mjs`（隔离签名测试源与两个 debug 构建）。

仓库已更名为 `panda-student/vault`。0.5.4 的 schema=2 清单指向新仓库的 ZIP；旧版会拒绝新清单，需要手动升级，不会把安装器误当主程序替换。保留 schema=1 读取能力仅用于已签名的历史 EXE 清单。

安装包采用 NSIS 3.13，使用现有工具链或校验固定 SHA-256 的便携编译器，不安装系统服务、不捆绑 WebView2。按当前 Windows 用户安装，默认 D:\vault，无 D 盘时回退当前用户本地目录；目录页允许修改，重装沿用已有目录。只写程序、授权文件、开始菜单快捷方式和当前用户卸载注册表。卸载明确删除安装文件，保留 data 和其他用户文件。

## 0.5.7 小写名称与覆盖安装

仓库、MCP 服务与配置键统一为 `vault`，默认目录 `D:\vault`，程序 `vault.exe`，分发包 `vault-<版本>.zip` / `vault-<版本>-setup.exe`。新版本仍读取历史大小写仓库 URL 和 ZIP 成员；旧更新器无法读取新的小写成员，0.5.6 及以前版本需要用本次安装包覆盖升级一次。

NSIS 先展开分发资源，再调用内嵌程序的私有安装引导入口。核心安装协调器复用 MCP Pause 协议，等待原请求完成、保留 stdio 连接；Windows 适配器只识别目标路径下当前用户的桌面进程，通过 Restart Manager 正常关闭，保存其数据目录。取得程序会话独占锁后，旧 EXE 改名保留，新 EXE 原子替换；桌面完成健康握手才恢复 MCP。失败时恢复旧 EXE、重启原数据目录，并恢复连接。安装器不打开密码库，也没有新增 MCP 安装或终止进程工具。UI 中未保存的修改应在安装前保存。

Windows 能力适配器与填写服务目前仍为不同模块；架构图里的合并是待实施的分层方案。
