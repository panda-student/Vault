# 便携版在线更新

0.5.1 起，设置页支持检查、下载、取消和重启更新。启动和每小时自动检查，由用户确认重启。0.4.x 首次升级需要手动下载一次。

## 模块与权限

桌面 update_action → CoreSession.execute(Operation) → 核心权限服务 → UpdateService。五种更新操作均 DesktopOnly；MCP 工具发现和核心执行均不允许更新。更新服务管理版本、下载、验签和就绪状态；Windows 能力服务负责隐藏辅助进程、等待退出与重启。辅助进程使用当前 EXE 的临时副本，无新增常驻程序。

所有桌面和 MCP 进程在打开数据库前按程序绝对路径获取共享文件锁。重启更新在核心操作屏障内申请独占锁，其他进程占用时提示退出/断开 MCP，不强制结束客户端。替换完成后使用原 EXE 路径、原绝对 --data-dir 重启。界面阻止未保存凭据、暂存草稿、权限草稿和保存中的操作丢失。

## 签名发布协议

稳定源：https://github.com/panda-student/local-password-manager/releases/latest/download/latest.json

本项目使用签名信封格式（不是 Tauri 安装版更新格式）：

```json
{"payload":"base64(UTF-8 JSON)","signature":"base64(Ed25519 signature over exact payload bytes)"}
```

签名载荷包含 schema=1、version、platform=windows-x86_64、固定仓库/版本目录的 Vault.exe URL、SHA-256、精确字节数和更新说明。先验签再处理版本，只接受较新的正式版本。下载限 64 MiB，校验长度、MZ 和哈希，安装前及辅助进程中再次校验。在线更新只替换 EXE，完整 ZIP 还包含 README、许可证和第三方声明。

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

正式发布需要修改三个版本文件及锁文件，再推送对应版本标签。Release 附件为 Vault.exe、vault-<version>.zip、latest.json、SHA256SUMS、size.json。Secret 缺失或不匹配时发布失败，不允许无签名降级。

## 数据与失败处理

更新不删除、移动或上传 data。数据库迁移仍由存储服务负责，沿用事务和升级前加密备份。旧程序保存在 .vault-update-<UUID>/previous.exe。新版须在 60 秒内完成核心、窗口与托盘初始化并回报令牌，否则终止本次启动的新版进程并尝试恢复旧 EXE。这不能证明全部业务功能正常。

EXE 回退不等于数据库回退。跨格式升级应保留迁移前备份，旧程序未必能读取未来格式。回退失败保留暂存目录供恢复。缓存位于 LocalAppData/LocalPasswordManager/updates，不进入发行包。

## 验证

Rust 测试覆盖签名、版本、哈希、跨进程锁和 MCP 固定禁止。原生验收使用两个独立构建、隔离数据目录、签名测试文件和实际窗口交互；报告放在忽略的 artifacts 验收目录。

仅 debug 构建接受 VAULT_UPDATE_TEST_SOURCE=http://127.0.0.1:<port>/latest.json；仍使用正式公钥验签。发行构建不接受更新地址或公钥的环境覆盖。

CI 迁移验收下载已发布的 v0.3.0 便携包并校验固定 SHA-256，覆盖真实旧版数据升级；本地仍可通过 VAULT_OLD_EXE 使用更早的 schema 1 版本。
