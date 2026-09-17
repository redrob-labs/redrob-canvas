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

데스크톱 셸은 Quick, Quick Controls 2, SVG를 포함한 Qt 6.8 이상이 필요합니다(Qt 6.12에서 검증).

```bash
cmake -S native -B build/qt -DCMAKE_BUILD_TYPE=Release \
  -DREDROB_ENABLE_GEGL=OFF -DREDROB_ENABLE_KRITA=OFF
cmake --build build/qt
ctest --test-dir build/qt --output-on-failure
./build/qt/qt/redrob-canvas
```

기본 빌드는 GEGL과 Krita에 대해 의존성이 없습니다. 어댑터를 켜는 의도는 명시적입니다. `REDROB_ENABLE_GEGL=ON`은 시스템에 `gegl-0.4 >= 0.4.66`을 요구하고, `REDROB_ENABLE_KRITA=ON`은 `REDROB_KRITA_SOURCE_DIR`가 정확히 커밋 `fdbf33b2146735465bb8aa59928fbc1890ceb160`이어야 합니다. 어느 쪽 스캐폴드를 켜도 제품 동작이 생기지는 않으며, 판단 기준은 언제나 capability 보고서입니다. 헤드리스로 capability만 확인하려면:

```bash
cmake -S native -B build/adapters -DREDROB_BUILD_QT=OFF
cmake --build build/adapters
ctest --test-dir build/adapters --output-on-failure
```

## 코어 파일 포맷

`redrob-core`는 RRG, PNG, JPEG, 무손실 WebP, OpenRaster, 그리고 의도적으로 제한한 결정적 SVG 부분집합에 대해 내용 기반으로 판별하는 타입 포맷 경계를 제공합니다. ABI v2 호환 심볼 `redrob_editor_import_file`과 `redrob_editor_export_file`이 이 포맷들을 엄격하고 한계가 정해진 JSON 옵션으로 라우팅하고, 실제 적용된 메타데이터와 기계가 읽을 수 있는 경고를 함께 돌려줍니다. 기본 옵션은 표현 손실을 거부합니다. JPEG의 알파 제거는 명시적인 불투명 매트나 불투명이 검증된 이미지를 요구하고, ORA/SVG 어댑터는 지원하지 않는 의미를 추측하지 않고 거부합니다. 기존 `save_project`/`load_project`, `import_png`/`export_png`와 그 C ABI 래퍼는 호환 동작을 유지한 채 그대로 사용할 수 있습니다.

Qt/QML 셸은 편집 가능한 RRG 프로젝트 정체성과 교환 포맷을 분리합니다. Open Project와 Save Project는 RRG 전용이고, Import와 Export Current Frame은 PNG, JPG/JPEG, 무손실 WebP, ORA, 제한된 SVG만 노출합니다. Import는 프로젝트 경로를 지우고, Export는 경로를 바꾸지도 타임라인을 이동하지도 않으며, 출력 게시는 `QSaveFile`을 사용하고, 손실 허용·JPEG 품질·불투명 매트 컨트롤이 명시적이라 조용한 품질 저하가 일어나지 않습니다. 정확한 경로와 한계는 `docs/formats.md`를 보세요.

## Redrob

클라이언트는 `https://console.redrob.ai/api/backend/v1`의 호스팅된 OpenAI 호환 스트리밍 API를 사용하고, 기본 모델은 `auto`이며, 함수 도구를 지원합니다. `REDROB_API_KEY`를 설정하거나 애플리케이션의 디바이스 인증 흐름으로 연결하세요.

## 배포 산출물

락파일 기반 고지 목록과 소스 제공 문서는 오직 `Cargo.lock`, 오프라인으로 고정된 Cargo 메타데이터, 로컬 크레이트 소스/라이선스 파일에서만 생성됩니다. 대응 소스 아카이브에는 락파일이 선택한 모든 레지스트리 패키지의 체크섬 검증 사본과 오프라인 Cargo 소스 치환 설정이 들어 있습니다.

```bash
python3 tools/generate_distribution_artifacts.py --check --gegl OFF --krita OFF
cmake --build build/qt --target redrob_source_bundle
cmake --install build/qt --prefix /desired/prefix
```

`redrob_distribution_check`는 독립 CMake 타깃으로도 실행할 수 있습니다. 설치 시 결정적 검사를 다시 돌리며 소스 번들, `THIRD_PARTY_NOTICES.md`, `SOURCE_OFFER.md`를 요구합니다. 의존성 상태 정책은 `docs/licensing.md`를 보세요.

## 릴리스

`vMAJOR.MINOR.PATCH` 태그를 푸시하면 `.github/workflows/release.yml`이 돕니다. 태그 커밋이 `main`에서 도달 가능한지 확인하고, 상류 핀·배포산출물 검사·포매팅·Clippy·테스트를 다시 돌린 뒤, 고정된 Qt 6.8 툴체인으로 Linux x86_64를 빌드하고 `ctest`를 실행합니다. 그리고 **초안** GitHub Release에 바이너리 tarball, 대응 소스 아카이브, `SOURCE_OFFER.md`, `THIRD_PARTY_NOTICES.md`, `LICENSE`, `COPYRIGHT`, SHA-256 합계 파일을 올립니다. 그 초안을 발행하는 것이 곧 릴리스이고, CDN이나 승격 단계는 없습니다.

이 프로젝트는 GPL-3.0-or-later이므로 대응 소스 없이 바이너리를 배포하는 것은 누락이 아니라 **라이선스 위반**입니다. 그래서 별도 잡이 그 자산들이 모두 첨부되지 않으면 릴리스를 실패시킵니다.

현재는 Linux x86_64만입니다. macOS와 Windows는 해당 러너에 Qt 툴체인과 플랫폼 서명이 필요해 별도 작업이고, Linux에서는 코드 서명 대상이 없으므로 서명 대신 공개된 체크섬으로 검증합니다.

## 라이선스

GPL-3.0-or-later. `LICENSE`는 4조가 요구하는 GNU General Public License 버전 3 전문이고, `COPYRIGHT`는 이 프로젝트의 저작권 표시와 보증 부인을 담습니다. `docs/licensing.md`, `THIRD_PARTY_NOTICES.md`, `SOURCE_OFFER.md`도 함께 보세요.
