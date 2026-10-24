# 安装、便携分发与在线更新

设置页支持检查、下载、取消和重启更新。启动和每小时自动检查，由用户确认重启。当前仅支持 schema=2 签名 ZIP 清单和小写固定路径，不读取历史 EXE 更新格式。

## 模块与权限

桌面 update_action → CoreSession.execute(Operation) → 核心权限服务 → UpdateService。五种更新操作均 DesktopOnly；MCP 工具发现和核心执行均不允许更新。更新服务管理版本、下载、验签和就绪状态；Windows 能力服务负责隐藏辅助进程、等待退出与重启。辅助进程使用当前 EXE 的临时副本，连接进程、共享核心及更新辅助进程均来自同一 EXE，不增加外部运行时。

桌面与 MCP 共享后台核心在打开数据库前按程序绝对路径获取共享文件锁。多个 stdio 连接进程通过当前 Windows 用户专用命名管道与桌面共用一份 Rust 后端（按 Windows 身份、EXE 绝对路径及规范化密码库目录隔离），不监听 TCP/HTTP 端口。共享核心按需启动，无客户端时空闲 5 秒退出。连接进程本身不打开数据库。

重启更新在核心操作屏障内发出暂停请求，等待所有连接已接收请求完成（最多 30 秒）；期间新请求明确返回 UPDATE_IN_PROGRESS，不排队、不重放写入。各连接关闭内部管道，共享核心释放存储与运行锁后，更新器才申请独占锁并替换程序。客户端的 stdio 进程与连接保留，更新成功或回退后重新连接唯一的新核心，内部恢复初始化握手。替换完成后使用原 EXE 路径、原绝对 --data-dir 重启。界面阻止未保存凭据、暂存草稿、权限草稿和保存中的操作丢失。

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

更新不删除、移动或上传 data。当前存储服务只接受 schema=4，运行时不迁移旧密码库。旧程序保存在 .vault-update-<UUID>/previous.exe。新版须在 60 秒内完成核心、窗口与托盘初始化并回报令牌，否则终止本次启动的新版进程并尝试恢复旧 EXE。这不能证明全部业务功能正常。

EXE 回退不等于数据库回退。新程序与旧程序必须使用同一数据格式，跨格式转换在程序外完成并保留加密备份。回退失败保留暂存目录供恢复。缓存位于 LocalAppData/LocalPasswordManager/updates，不进入发行包。

## 验证

Rust 测试覆盖签名、版本、哈希、跨进程锁和 MCP 固定禁止。原生验收使用两个独立构建、隔离数据目录、签名测试文件和实际窗口交互；报告放在忽略的 artifacts 验收目录。

仅 debug 构建接受 VAULT_UPDATE_TEST_SOURCE=http://127.0.0.1:<port>/latest.json；仍使用正式公钥验签。发行构建不接受更新地址或公钥的环境覆盖。

CI 验证当前格式的 MCP、权限和共享后端，安装交接测试使用本次构建的程序，不再下载旧版迁移样本。

命名管道设置显式受保护 DACL（当前 Windows 用户与 LocalSystem），拒绝远程连接；连接端校验管道服务端进程所属 Windows 身份。暂停请求只包含随机令牌与到期时间，不包含凭据、操作或客户端配置；失败路径按令牌解除暂停，辅助进程异常终止的遗留暂停请求 5 分钟后失效。

共享核心与多连接验收：`node scripts/verify-mcp-handoff.mjs`（可用 VAULT_EXE 指定程序）；更新/回退和连接保留验收：`node scripts/verify-update.mjs`（隔离签名测试源与两个 debug 构建）。

仓库已更名为 `panda-student/vault`。0.5.4 的 schema=2 清单指向新仓库的 ZIP；旧版会拒绝新清单，需要手动升级，不会把安装器误当主程序替换。程序不再读取 schema=1 清单。

安装包采用 NSIS 3.13，使用现有工具链或校验固定 SHA-256 的便携编译器，不安装系统服务、不捆绑 WebView2。按当前 Windows 用户安装，默认 D:\vault，无 D 盘时回退当前用户本地目录；目录页允许修改，重装沿用已有目录。只写程序、授权文件、开始菜单快捷方式和当前用户卸载注册表。卸载明确删除安装文件，保留 data 和其他用户文件。

## 0.5.8 小写名称与覆盖安装

仓库、MCP 服务与配置键统一为 `vault`，默认目录 `D:\vault`，程序 `vault.exe`，分发包 `vault-<版本>.zip` / `vault-<版本>-setup.exe`。更新器仅接受当前仓库的规范 URL 和小写 ZIP 成员，不保留历史仓库、大小写成员或独立 EXE 更新分支。

NSIS 先展开分发资源，再调用内嵌程序的私有安装引导入口。核心安装协调器复用 MCP Pause 协议，等待原请求完成、保留 stdio 连接；Windows 适配器只识别目标路径下当前用户的桌面进程，通过 Restart Manager 正常关闭，保存其数据目录。取得程序会话独占锁后，旧 EXE 改名保留，新 EXE 原子替换；桌面完成健康握手才恢复 MCP。失败时恢复旧 EXE、重启原数据目录，并恢复连接。安装器不打开密码库，也没有新增 MCP 安装或终止进程工具。UI 中未保存的修改应在安装前保存。

填写服务负责凭据解析与编排，Windows 能力服务负责原生定位、输入与文件替换；两者的调用关系已在代码和架构图中同步。

## 当前单后端宿主

桌面和 MCP 共用同一宿主中的核心服务。在线更新先排空 MCP 请求，再取得共享核心的独占执行门，避免升级与排队操作互相等待。覆盖安装时，MCP 排空后保留有窗口的宿主供安装器自动关闭；没有窗口的宿主可直接退出。新版健康确认必须晚于桌面窗口创建和连接监听就绪，避免暂停标记使尚未完成启动的宿主提前退出。

`verify-update.mjs` 支持 `UPDATE_FROM_VERSION`、`UPDATE_TO_VERSION`、`UPDATE_FAILED_VERSION` 和对应的两个隔离 debug 构建；验收包含签名 ZIP 实际升级、启动失败回退、单一宿主 PID 变化和原 MCP 连接进程保持不变。当前协议没有增加对 MCP 的更新或桌面权限入口。健康启动中的 setup 错误正常退出，由协调器负责恢复；不让框架把预期的健康失败变成崩溃并触发额外重启。NSIS 直接等待协调器退出码。

## 0.6.0 格式边界

本版本仅接受 schema=4 密码库，移除旧迁移、group 别名、历史 API 协议与 fill_credential 工具；窗口和文件写入统一使用 write_credential。网址和 API 服务地址支持打开浏览器，API 凭据另有文档地址。部署前须在程序外备份和转换旧库，并验证凭据、标签和权限完整；不能直接将旧库交给新程序自动迁移。
