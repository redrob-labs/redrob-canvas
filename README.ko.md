# Redrob Canvas

[English](./README.md) · **한국어**

Redrob Canvas는 Rust와 Qt Quick으로 만든 크로스플랫폼 에이전틱 그래픽 편집기입니다.

redrob-canvas는 UI 포크가 아니라 새로 설계한 제품 구조입니다. Krita와 GIMP는 실행 가능한 기준점으로 커밋에 고정된 채 남아 있고, 검증된 그 동작을 타입이 있는 Rust 커맨드 시스템과 현대적인 QML 경험 뒤로 옮깁니다. 페인팅 동작은 Krita를, 이미지 처리 동작은 GIMP/GEGL을 따르며, 제품 UX와 에이전트 워크플로는 redrob-canvas의 것입니다.

## 프로젝트 구조

```text
crates/
  redrob-core/       문서, 레이어, 래스터 엔진, 선택, 커맨드, 히스토리
  redrob-agent/      Redrob 스트리밍 API, 도구 루프, 승인
  redrob-ffi/        Qt 호스트가 사용하는 안정 C ABI
native/qt/            Qt 6 호스트, QObject 모델, 캔버스 브리지
qml/                  현대적인 Qt Quick 인터페이스
docs/                 아키텍처, 호환성 표, 고정된 상류 목록
```

## 아키텍처 규칙

1. QML과 에이전트는 **같은** 타입 커맨드를 제출합니다.
2. 문서를 바꾸는 모든 커맨드는 트랜잭션이며, 감사 가능하고, 되돌릴 수 있습니다.
3. 렌더 스레드는 불변 스냅샷만 소비하고 문서 상태를 소유하지 않습니다.
4. 상류 동작은 커밋으로 고정하고 기능 단위로 추적합니다.
5. Krita/GIMP 코드는 명시적 어댑터 뒤에서만 통합하며, 상류 UI는 가져오지 않습니다.
6. 파괴적 작업과 에이전트가 생성한 작업은 미리보기 또는 명시적 커밋 경계를 요구합니다.

## 빌드

코어와 프로토콜 검사는 Qt가 필요 없습니다.

```bash
cargo test --workspace
```

데스크톱 셸은 Quick, Quick Controls 2, SVG를 포함한 Qt 6.11 이상이 필요합니다.

```bash
cmake -S native -B build/qt -DCMAKE_BUILD_TYPE=Release
cmake --build build/qt
ctest --test-dir build/qt --output-on-failure
./build/qt/qt/redrob-canvas
```

앱은 GIMP, GEGL, Krita 코드를 링크하지 않습니다. 그 동작은 Rust로 다시 구현했습니다. 선택 네이티브 어댑터는 babl과 Little-CMS 2(`REDROB_ENABLE_BABL`, `REDROB_ENABLE_LCMS`, 기본값 OFF)뿐이며, 판단 기준은 언제나 capability 보고서입니다. 헤드리스로 capability만 확인하려면:

```bash
cmake -S native -B build/adapters -DREDROB_BUILD_QT=OFF
cmake --build build/adapters
ctest --test-dir build/adapters --output-on-failure
```

## 코어 파일 포맷

`redrob-core`는 RRG, PNG, JPEG, 무손실 WebP, OpenRaster, 그리고 의도적으로 제한한 결정적 SVG 부분집합에 대해 내용 기반으로 판별하는 타입 포맷 경계를 제공합니다. ABI v2 호환 심볼 `redrob_editor_import_file`과 `redrob_editor_export_file`이 이 포맷들을 엄격하고 한계가 정해진 JSON 옵션으로 라우팅하고, 실제 적용된 메타데이터와 기계가 읽을 수 있는 경고를 함께 돌려줍니다. 기본 옵션은 표현 손실을 거부합니다. JPEG의 알파 제거는 명시적인 불투명 매트나 불투명이 검증된 이미지를 요구하고, ORA/SVG 어댑터는 지원하지 않는 의미를 추측하지 않고 거부합니다. 기존 `save_project`/`load_project`, `import_png`/`export_png`와 그 C ABI 래퍼는 호환 동작을 유지한 채 그대로 사용할 수 있습니다.

Qt/QML 셸은 편집 가능한 RRG 프로젝트 정체성과 교환 포맷을 분리합니다. 위쪽 바의 Open, Save, Save As는 RRG 전용이고, Import와 Export(현재 프레임)는 PNG, JPG/JPEG, 무손실 WebP, ORA, 제한된 SVG만 노출합니다. Import는 프로젝트 경로를 지우고, Export는 경로를 바꾸지도 타임라인을 이동하지도 않으며, 출력 게시는 `QSaveFile`을 사용하고, 손실 허용·JPEG 품질·불투명 매트 컨트롤이 명시적이라 조용한 품질 저하가 일어나지 않습니다. 정확한 경로와 한계는 `docs/formats.md`를 보세요.

## Redrob

**Redrob Code가 필요합니다.** 앱 안의 에이전트는 이 컴퓨터에 하나 있는 공유 Redrob Code 데몬(`~/.redrob/run/server.json`; 없으면 `redrob service start`로 띄움)의 `<daemon>/v1/chat/completions`에 말합니다. OpenAI 호환 스트리밍이고 기본 모델은 `auto`, 함수 도구를 지원합니다. 모든 요청에 데몬 자체의 인증, `x-redrob-app: canvas`, 그리고 문서 id가 `x-redrob-session`으로 실립니다. Redrob 콘솔 키는 데몬이 보관하고 데몬이 그 키로 콘솔을 부릅니다. 이 앱은 자체 키를 갖지 않고, 채팅을 위해 콘솔로 직접 가는 경로도 없습니다 -- 마스킹이 적용되는 곳이 한 곳이 되도록. 로그인은 한 번만: Redrob Code에서, Redrob Launcher에서, 또는 여기서. 에이전트 패널의 "Connect to Redrob console"은 콘솔의 디바이스 인증 흐름을 돌리고 승인된 키를 엔진에 넘깁니다(`PUT /auth/redrob`). 패널은 엔진이 로그인돼 있는지 `GET /provider`에서 읽고, 엔진이 쓰는 키가 런처의 것이면 "Redrob Launcher로 로그인됨"이라고 말합니다. Redrob Code가 없으면 패널은 "Redrob Code is not installed. Install it with Redrob Launcher."와 [런처 최신 릴리스](https://github.com/redrob-labs/redrob-launcher/releases/latest) 링크를 보이고, 결정적 로컬 제안 모드가 대신 돕니다. 이전 버전의 이 앱이 자기 설정 폴더에 저장했던 키는 한 번 엔진에 넘기고 지웁니다. 데몬 레코드와 바이너리는 `redrob-engine-client`(redrob-code `docs/LOCAL-ENGINE-API.md` 3절)가 찾고, `REDROB_RUN_DIR`와 `REDROB_CODE_BIN`은 테스트를 비공개 데몬으로 향하게 합니다.

## 배포 산출물

락파일 기반 고지 목록과 소스 제공 문서는 오직 `Cargo.lock`, 오프라인으로 고정된 Cargo 메타데이터, 로컬 크레이트 소스/라이선스 파일에서만 생성됩니다. 대응 소스 아카이브에는 락파일이 선택한 모든 레지스트리 패키지의 체크섬 검증 사본, git으로 고정된 모든 패키지를 락파일이 지정한 커밋에서 검증한 체크아웃 소스(저장소 루트의 라이선스 텍스트 포함), 그리고 오프라인 Cargo 소스 치환 설정이 들어 있습니다.

```bash
python3 tools/generate_distribution_artifacts.py --check --gegl OFF --krita OFF
cmake --build build/qt --target redrob_source_bundle
cmake --install build/qt --prefix /desired/prefix
```

`redrob_distribution_check`는 독립 CMake 타깃으로도 실행할 수 있습니다. 설치 시 결정적 검사를 다시 돌리며 소스 번들, `THIRD_PARTY_NOTICES.md`, `SOURCE_OFFER.md`를 요구합니다. 의존성 상태 정책은 `docs/licensing.md`를 보세요.

## 로드맵

앞으로 무엇을 어떤 순서로 만드는지는 [ROADMAP.md](ROADMAP.md)에 있습니다.

## 릴리스

`main`에 `vMAJOR.MINOR.PATCH` 태그를 푸시한 뒤 `gh workflow run release.yml --ref main -f tag=vMAJOR.MINOR.PATCH`를 실행합니다. 워크플로(`.github/workflows/release.yml`)는 태그를 `origin/main`·`Cargo.toml`·체인지로그와 대조하고, 모든 플랫폼을 GitHub 러너에서 `aqtinstall`의 Qt 6.11.2로 빌드·테스트한 다음, macOS는 조직의 Developer ID로 서명·공증하고, Windows는 잡의 OpenID Connect 로그인으로 Azure Artifact Signing에서 서명합니다(인증서 파일도 클라이언트 시크릿도 없습니다. 앱을 먼저 서명한 뒤 zip으로 묶고, 제거 프로그램, 설치 파일 순으로 서명합니다). 그 결과물은 공유 redrob-labs 발행 잡(`redrob-labs/.github/.github/workflows/release-publish.yml`)이 받아 `SHA256SUMS.txt`와 `redrob-launcher.json`을 쓰고 릴리스를 latest로 **발행**합니다. `gh workflow run release.yml --ref <branch> -f dry_run=true`는 지정한 브랜치를 빌드·서명만 하고 아무것도 발행하지 않습니다. 풀 리퀘스트는 공유 gitflow 규칙(`redrob-labs/.github/.github/workflows/gitflow.yml`)이 지킵니다: 브랜치 이름은 조직 공통 타입 목록의 `<type>/<slug>`이고, `main`은 `develop`에 포함돼 있어야 합니다. `tools/release/`의 로컬 빌드 스크립트는 우리 기계에서 빌드를 리허설하는 용도로 남아 있으며 릴리스 경로는 아닙니다.

어떤 플랫폼을 빌드하는지는 `tools/release_matrix.py`가 결정하고, 검증 잡이 요구하는 목록과 릴리스 노트가 주장하는 목록도 같은 리스트에서 나옵니다 — 그래서 노트가 빌드되지 않은 바이너리를 약속할 수 없습니다. **Windows x64는 리포지터리 변수 `WINDOWS_SIGNING_READY`가 `"true"`가 된 뒤에만 빌드됩니다**. 그때까지는 서명 없는 실행 파일을 내보내는 대신 실행 로그에 이유를 남기고 건너뜁니다. Linux x86_64와 macOS arm64는 항상 빌드됩니다.

Qt 핀은 최신 브랜치가 아니라 정착된 브랜치의 최신 패치입니다: 6.12.0은 릴리스 며칠 뒤에도 `download.qt.io`에 Windows 아카이브 체크섬이 없어 모든 플랫폼 레그가 실패했습니다. `redrob_qml_restricted_lint`는 qmllint에 `--only-explicit-categories` 지원 여부를 물어보고 있을 때만 사용하므로, Qt 6.12 이상에서는 더 엄격한 린트가 적용되면서도 그것이 빌드 필수 조건이 되지는 않습니다.

| 플랫폼 | 빌드 | 서명 |
| --- | --- | --- |
| Linux x86_64 | Ninja, Qt 동봉(`tools/release/bundle_linux_qt.py`), `.deb`와 `.tar.gz` | 없음 — Linux에는 코드 서명 대상이 없으므로 공개된 SHA-256 합계로 검증 |
| macOS arm64 (Apple 실리콘) | Ninja, `macdeployqt`, `.dmg`(`installer/macos/`)와 `.zip` | Developer ID + hardened runtime, App Store Connect API 키로 공증 후 staple — 앱과 디스크 이미지 각각, `spctl`로 Gatekeeper 판정 확인 |
| Windows x64 | Ninja + MSVC, `windeployqt`, `-setup.exe`(`installer/windows/`)와 `-portable.zip` | `redrob-labs/.github/actions/sign-windows`로 Azure Artifact Signing(OIDC): 앱·제거 프로그램·설치 파일에 Authenticode SHA-256 + RFC 3161 타임스탬프, `signtool verify /pa`가 Valid이고 서명자 CN이 `McKinley Rice, Inc.`여야 함 |

### 설치

| 플랫폼 | 받을 파일 | 설치 | 제거 |
| --- | --- | --- | --- |
| Windows | `…-windows-x64-setup.exe` | 실행합니다. 사용자별 설치라 관리자 권한이 필요 없고, 시작 메뉴와 바탕화면에 바로가기가 생깁니다 | 설정 > 앱 > Redrob Canvas, 또는 설치 폴더의 `Uninstall Redrob Canvas.exe` |
| macOS | `…-macos-arm64.dmg` | 열어서 Redrob Canvas를 Applications로 끌어 놓습니다 | 디스크 이미지의 `Uninstall Redrob Canvas`를 엽니다 |
| Linux | `…-linux-x86_64.deb` | `sudo apt install ./redrob-canvas-…-linux-x86_64.deb` | `sudo apt remove redrob-canvas` |

두 제거 프로그램 모두 먼저 묻고, 지우라고 하지 않으면 설정은 남깁니다. 실행 중인 Canvas를 닫지 않습니다. 손으로 풀고 싶은 분을 위해 `-portable.zip`과 `.tar.gz`도 계속 올리고, Redrob Launcher는 Windows에서 그 `-portable.zip`을, macOS에서 `.dmg`를, Linux에서 `.tar.gz`를 설치합니다.

macOS 빌드는 arm64 전용이고 인텔 Mac은 지원하지 않습니다. 서명 자료는 org 시크릿에서 오고, 자격 증명이 없으면 서명 없는 산출물을 발행하는 대신 빌드를 실패시킵니다.

이 프로젝트는 GPL-3.0-or-later이므로 대응 소스 없이 바이너리를 배포하는 것은 누락이 아니라 **라이선스 위반**입니다. 모든 아카이브가 바이너리와 함께 `LICENSE`, `COPYRIGHT`, `THIRD_PARTY_NOTICES.md`, `SOURCE_OFFER.md`를 담고, 대응 소스 아카이브가 릴리스에 첨부되며, 그 실행이 빌드한 모든 플랫폼의 아카이브와 위 라이선스 자산이 전부 있지 않으면 별도 잡이 릴리스를 실패시킵니다.

## 라이선스

GPL-3.0-or-later. `LICENSE`는 4조가 요구하는 GNU General Public License 버전 3 전문이고, `COPYRIGHT`는 이 프로젝트의 저작권 표시와 보증 부인을 담습니다. `docs/licensing.md`, `THIRD_PARTY_NOTICES.md`, `SOURCE_OFFER.md`도 함께 보세요.
