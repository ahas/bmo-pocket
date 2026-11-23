[English](README.md) | **한국어** | [日本語](README.ja.md) | [简体中文](README.zh-CN.md)

# pocket

도구 호스트 `bmo-pocket`입니다. 하나의 프로세스가 모든 도구 세션의 MCP 도구를 제공합니다. 도구가 자체 저장소에
있든 서비스 저장소의 `tools/` 폴더에 있든 각 도구의 `tool.toml` 매니페스트를 읽고, 세션 레코드에 담긴 네임스페이스,
권한(capability), 허가(grant)에 따라 그 항목을 제공합니다. 세션에는 상주 도구 몇 개만 이름으로 보이고, 나머지 모든
도구는 툴북을 통해 찾아 씁니다. 그래서 도구를 추가하거나 빼도 실행 중인 세션의 프롬프트 첫머리는 바뀌지 않습니다.
하네스는 HTTP로, 또는 하네스 플러그인 `bmo-pocket mcp`를 통해 호스트에 접근합니다.

이 저장소에는 도구 SDK도 있습니다. 모든 도구가 따르는 계약, 곧 호스트가 읽는 호출 프로토콜과 매니페스트 스키마와,
TypeScript나 Python으로 작성한 도구가 지니고 다니는 파일 하나짜리 헬퍼입니다.

도구를 만들고 시험하고 배포 없이 올리는 방법은 `docs/authoring.md`에 있습니다.

## 구성

| 폴더 | 내용 |
|---|---|
| `client/` | 제어 소켓의 클라이언트입니다. `docs/openapi.json`을 생성하는 Rust 크레이트 `bmo-pocket-client`와, 그 문서에서 생성한 TypeScript 패키지 `@bmo/pocket-client`가 있습니다 |
| `console/` | 호스트의 화면을 담은 콘솔 번들 `bmo-pocket-console`입니다. 들여온 고정 bmo-ui 릴리스 위에 만들어져 콘솔 화면이 제공하는 파일로 빌드되며, 매니페스트에 대한 호스트의 선언도 여기 있습니다(`docs/api.md`, 「The console face」) |
| `deploy/` | 배포가 배포 트리에서 링크하는 호스트의 사용자 유닛 `bmo-mcp-host.service`입니다 |
| `docs/` | 기준 문서입니다. 도구 만들기(`authoring.md`), 호스트(`mcp-host.md`), 툴북(`toolbook.md`), 레지스트리(`registry.md`), 다른 모든 유닛이 맞추는 경계(`api.md`)와 그 OpenAPI 문서(`openapi.json`)가 있습니다 |
| `examples/` | Rust, TypeScript, Python으로 각각 작성한 예제 도구 하나이며, 저장소의 테스트가 각각을 올리고 호출합니다 |
| `host/` | 도구 호스트 `mcp-host`와 명령 `bmo-pocket`(레지스트리의 서브커맨드와 `mcp`)이며, 테스트는 `host/tests/`에 있습니다 |
| `plugin/` | 하네스 플러그인 `bmo-pocket-plugin`입니다. `bmo-pocket mcp`가 제공하는 MCP 서버와 `bmo-pocket` 스킬이 있습니다 |
| `prompts/` | 호스트가 제공하는 프롬프트 텍스트 `tool-use.md`입니다. 세션의 시스템 프롬프트에 들어가는, 도구를 찾고 쓰는 방법에 관한 단락입니다 |
| `sdk/` | 도구 SDK입니다. 호스트가 읽는 호출 프로토콜과 매니페스트 스키마를 담은 `bmo-tool` 크레이트 `sdk/contract/`, 그리고 TypeScript나 Python 도구가 가져다 쓰는 헬퍼 `sdk/ts-kit/`과 `sdk/py-kit/`이 있습니다 |
| `tests/` | 저장소 전체에 걸친 검사와, 설치된 CLI가 호스트에서 읽어 가는 것에 호스트를 맞추는 `tests/host-contract/`가 있습니다 |

## 개인정보

이 저장소에는 개인정보가 없습니다. `PERSONAL-DATA.md`는 이곳에 절대 들어가면 안 되는 것과 이를 지키는 검사를
설명합니다. testbed의 `personal-data` 검사는 추적되는 모든 파일을 읽고, 메일 주소, 홈 경로, 전화번호, 토큰,
사설 네트워크 주소를 발견하면 값은 보여 주지 않고 파일, 줄, 종류만 알려 줍니다.

## 빌드와 테스트

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

## 버전

모든 크레이트와 패키지가 하나의 버전을 씁니다. 첫 릴리스 전까지는 `0.1.0`에 리비전을 붙인 `0.1.0-rev.<n>`이며,
`docs/api.md`의 「Version」에 적힌 대로입니다.
