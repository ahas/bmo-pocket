[English](README.md) | [한국어](README.ko.md) | [日本語](README.ja.md) | **简体中文**

# pocket

工具宿主 `bmo-pocket`：由一个进程为所有工具会话提供 MCP 工具。无论工具位于独立仓库，还是位于某个服务仓库的
`tools/` 目录中，它都会读取每个工具的 `tool.toml` 清单，并按照会话记录所携带的命名空间、能力（capability）和授权
（grant）提供其中的条目。会话只会按名称看到少数常驻工具，其余所有工具都通过工具手册（toolbook）找到，因此增加或
移除工具都不会改变正在运行的会话的提示词开头。harness 通过 HTTP，或通过其 harness 插件 `bmo-pocket mcp` 访问它。

本仓库还包含工具 SDK：所有工具都遵循的契约，即宿主所读取的调用协议和清单 schema，以及用 TypeScript 或 Python
编写的工具随身携带的单文件辅助库。

如何制作工具、测试它，并在无需部署的情况下让它上线，见 `docs/authoring.md`。

## 目录结构

| 目录 | 内容 |
|---|---|
| `client/` | 控制 socket 的客户端：生成 `docs/openapi.json` 的 Rust crate `bmo-pocket-client`，以及由该文档生成的 TypeScript 包 `@bmo/pocket-client` |
| `console/` | 宿主的界面，作为控制台包 `bmo-pocket-console`，基于其内置的固定 bmo-ui 发布版本，构建为控制台界面所提供的文件；以及宿主对其清单的声明（`docs/api.md`，「The console face」） |
| `deploy/` | 宿主的用户单元 `bmo-mcp-host.service`，部署时从部署树中链接 |
| `docs/` | 规范文档：制作工具（`authoring.md`）、宿主（`mcp-host.md`）、工具手册（`toolbook.md`）、注册表（`registry.md`）、公司共享个人应用的目录（`catalogue.md`）、其他所有单元都要对齐的边界（`api.md`）及其 OpenAPI 文档（`openapi.json`） |
| `examples/` | 分别用 Rust、TypeScript 和 Python 编写的示例工具，仓库的测试会逐一注册并调用它们 |
| `host/` | 工具宿主 `mcp-host` 和命令 `bmo-pocket`（注册表的子命令和 `mcp`），测试位于 `host/tests/` |
| `plugin/` | harness 插件 `bmo-pocket-plugin`：由 `bmo-pocket mcp` 提供的 MCP 服务器及其 `bmo-pocket` skill |
| `prompts/` | 宿主提供的提示词文本 `tool-use.md`：会话系统提示词中关于查找和使用工具的段落 |
| `sdk/` | 工具 SDK：`sdk/contract/`，即包含宿主所读取的调用协议和清单 schema 的 `bmo-tool` crate；`sdk/ts-kit/` 和 `sdk/py-kit/`，供 TypeScript 或 Python 工具导入的辅助库 |
| `tests/` | 跨整个仓库的检查，以及 `tests/host-contract/`，用于让宿主符合已安装的 CLI 从它读取的内容 |

## 个人数据

本仓库不包含任何个人数据。`PERSONAL-DATA.md` 说明了哪些内容绝不能出现在这里，以及守住这一点的检查：testbed 的
`personal-data` 检查会读取每个被跟踪的文件，一旦发现邮件地址、主目录路径、电话号码、令牌或私有网络地址，便指出
文件、行号和类别，但从不显示其值。

## 构建与测试

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
(cd sdk/ts-kit && bun test)
(cd client && bun test)
(cd console && bun install && bun run check)
(cd tests/host-contract && bun test)
```

## 版本

版本是整个套件的版本，在构建时从 git 标签读取；crate 和包的版本字段是从不变动的占位值，如 `docs/api.md` 的
「Version」 所述。
