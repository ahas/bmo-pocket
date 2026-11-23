[English](README.md) | [한국어](README.ko.md) | **日本語** | [简体中文](README.zh-CN.md)

# pocket

ツールホスト `bmo-pocket` です。一つのプロセスで、すべてのツールセッションの MCP ツールを提供します。ツールが独自の
リポジトリにあってもサービスリポジトリの `tools/` フォルダーにあっても、各ツールの `tool.toml` マニフェストを読み、
セッションのレコードが持つネームスペース、ケイパビリティ、グラントに従ってそのエントリーを提供します。セッションには
常駐ツールがいくつか名前で示されるだけで、それ以外のツールにはすべてツールブックを通じてたどり着きます。そのため、
ツールを追加したり外したりしても、実行中のセッションのプロンプトの冒頭は変わりません。ハーネスは HTTP で、または
ハーネスプラグイン `bmo-pocket mcp` を通じてホストにアクセスします。

このリポジトリにはツール SDK も含まれています。すべてのツールが従う契約、つまりホストが読む呼び出しプロトコルと
マニフェストスキーマと、TypeScript や Python で書かれたツールが持ち歩く 1 ファイルのヘルパーです。

ツールを作り、試し、デプロイなしで公開する方法は `docs/authoring.md` にあります。

## 構成

| フォルダー | 内容 |
|---|---|
| `client/` | 制御ソケットのクライアントです。`docs/openapi.json` を生成する Rust クレート `bmo-pocket-client` と、そこから生成される TypeScript パッケージ `@bmo/pocket-client` があります |
| `console/` | ホストの画面をまとめたコンソールバンドル `bmo-pocket-console` です。取り込んだ固定の bmo-ui リリースの上に作られ、コンソール画面が配信するファイルとしてビルドされます。マニフェストに関するホストの宣言もここにあります(`docs/api.md`、「The console face」) |
| `deploy/` | デプロイがデプロイツリーからリンクするホストのユーザーユニット `bmo-mcp-host.service` です |
| `docs/` | 正典となるドキュメントです。ツールの作り方(`authoring.md`)、ホスト(`mcp-host.md`)、ツールブック(`toolbook.md`)、レジストリ(`registry.md`)、他のすべてのユニットが合わせる境界(`api.md`)とその OpenAPI ドキュメント(`openapi.json`)があります |
| `examples/` | Rust、TypeScript、Python でそれぞれ書かれた例のツールで、リポジトリのテストがそれぞれを登録して呼び出します |
| `host/` | ツールホスト `mcp-host` とコマンド `bmo-pocket`(レジストリのサブコマンドと `mcp`)で、テストは `host/tests/` にあります |
| `plugin/` | ハーネスプラグイン `bmo-pocket-plugin` です。`bmo-pocket mcp` が提供する MCP サーバーと `bmo-pocket` スキルがあります |
| `prompts/` | ホストが配信するプロンプトテキスト `tool-use.md` です。セッションのシステムプロンプトに含まれる、ツールの探し方と使い方の段落です |
| `sdk/` | ツール SDK です。ホストが読む呼び出しプロトコルとマニフェストスキーマを持つ `bmo-tool` クレート `sdk/contract/`、そして TypeScript や Python のツールがインポートするヘルパー `sdk/ts-kit/` と `sdk/py-kit/` があります |
| `tests/` | リポジトリ全体にまたがる検査と、インストール済みの CLI がホストから読み取る内容にホストを合わせる `tests/host-contract/` があります |

## 個人データ

このリポジトリに個人データは含まれていません。`PERSONAL-DATA.md` は、ここに決して置いてはならないものと、それを
守る検査を説明しています。testbed の `personal-data` チェックは追跡中のすべてのファイルを読み、メールアドレス、
ホームパス、電話番号、トークン、プライベートネットワークのアドレスを見つけると、値は出さずにファイル、行、種類を
示します。

## ビルドとテスト

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

## バージョン

すべてのクレートとパッケージが一つのバージョンを共有します。最初のリリースまでは `0.1.0` にリビジョンを付けた
`0.1.0-rev.<n>` で、`docs/api.md` の「Version」に記載のとおりです。
