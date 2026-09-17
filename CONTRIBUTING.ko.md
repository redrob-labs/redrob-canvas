# 기여 가이드

[English](./CONTRIBUTING.md) · **한국어**

도와주셔서 감사합니다. 이 문서는 저장소의 작업 규약입니다. 브랜치를 어떻게 이름 짓는지, 머지
전에 무엇이 초록불이어야 하는지, 그리고 이 저장소를 특별하게 만드는 한 가지를 다룹니다.

## 무엇이고, 무엇이 아닌가

레드롭 캔버스는 **UI 포크가 아니라 새로운 제품 아키텍처입니다.** Krita와 GIMP는 동작의 실행 가능한
기준으로 핀되어 남아 있고, 그 동작은 타입이 있는 Rust 커맨드 시스템과 QML 표면 뒤로 옮겨집니다.

책임 분담은 반드시 분명히 지켜야 합니다.

- **페인팅 동작은 Krita를 따릅니다.** 둘이 다르면 Krita가 맞고 버그는 우리 것입니다.
- **이미지 처리는 GIMP와 GEGL을 따릅니다.**
- **제품 UX와 에이전트 워크플로는 우리 것이고**, 둘 중 어느 쪽에도 빚지지 않습니다.

그래서 브러시 동작을 바꾸는 pull request는 어느 상류를 기준으로 확인했는지 밝혀야 합니다.
"이게 더 느낌이 좋다"는 핀된 기준에서 벗어날 이유가 되지 않습니다. 기준을 실제로 측정해서 다르다는
것을 확인한 것이 이유가 됩니다.

## 브랜치 모델

오래 사는 브랜치는 둘입니다. `develop`은 작업이 들어오는 곳이고, `main`은 릴리스된 상태입니다.

- **`develop`**은 기본 브랜치이자 통합 브랜치입니다. 여기서 작업 브랜치를 떼고, pull request도 여기로
  다시 엽니다. 저장소를 클론하면 `develop`에 있습니다.
- **`main`**은 릴리스된 상태입니다. `release/*`와 `hotfix/*` 브랜치에서 온 pull request만 받고,
  릴리스 태그는 여기서 뜹니다. 그 밖의 어떤 것도 여기로 머지되지 않습니다.
- **작업 브랜치**는 `develop`에서 뗀 `<type>/<short-slug>` 형식입니다. 예: `fix/svg-namespace`, `feat/layer-blend`. 타입은 `feat`,
  `fix`, `chore`, `docs`, `test`, `refactor`, `perf`입니다.
- **머지는 squash만 합니다**, 그리고 머지할 때 브랜치는 삭제됩니다. pull request 하나가 커밋 하나가
  되므로 `git log develop`은 그래프가 아니라 변경 목록으로 읽힙니다. squash 커밋의 본문은 작업 중에
  쓴 메시지들을 이어붙인 것이 아니라 pull request 본문입니다.

이 목록은 리뷰하는 사람에게 맡기지 않고 `.github/workflows/gitflow.yml`이 검사합니다. **branch name
follows the convention** 잡은 승격이나 back-merge로 pull request에 들어오는 `develop`과 `main`은 통과
시키고, 그 밖에는 `<type>/<short-slug>` 형식에 타입이 `feat`, `fix`, `chore`, `docs`, `test`,
`refactor`, `perf`, `release`, `hotfix` 중 하나여야 합니다. 브랜치 이름은 무엇이 그것을 만들었는지가
아니라 변경이 무엇인지를 말하므로, `kiro/`처럼 도구나 에이전트 이름을 접두로 쓰면 실패합니다. 두 번째
잡은 `main`으로의 푸시에서 돌고, `main`이 `develop`에 없는 커밋을 갖고 있는 동안 실패합니다. 그
back-merge가 바로 건너뛰어지는 단계이고, 그러면 기본 브랜치에서 뗀 브랜치가 릴리스된 수정을 조용히
빠뜨리기 때문입니다.

두 브랜치 모두 이 문서가 아니라 GitHub ruleset이 강제합니다.

- 직접 푸시 금지 — 모든 변경은 pull request로 들어옵니다;
- force push 금지, 브랜치 삭제 금지;
- linear history;
- 필수 상태 검사가 통과해야 합니다 — 가장 최신 팁을 기준으로 통과할 필요는 없으므로, 봇 업데이트가
  줄지어 있어도 하나씩 리베이스하고 다시 돌릴 필요가 없습니다;
- 리뷰 스레드는 머지 전에 해결되어야 합니다.

ruleset은 pull request가 *어느* 브랜치에서 왔는지를 표현할 수 없으므로, "`release/*`와 `hotfix/*`만
`main`으로 머지된다"는 이 문서가 담고 리뷰어가 지키는 관례입니다. 그중 한 부분은 기계가 검사합니다.
릴리스 워크플로는 커밋이 `origin/main`에서 도달할 수 없는 태그의 빌드를 거부하므로, `develop`에서
바로 태그를 뜨면 출시되는 대신 실패합니다.

### 릴리스하기

```bash
git switch develop && git pull
git switch -c release/v0.2.0
# bump the version, update the changelog, run the release check
# open a pull request into main and merge it, then tag main:
git switch main && git pull
git tag -a v0.2.0 -m "Redrob Canvas v0.2.0"
git push origin v0.2.0
# bring main's release commit back so develop does not fall behind:
git switch -c chore/sync-main-to-develop main
# open a pull request into develop
```

핫픽스는 같은 모양인데 `hotfix/*`를 `develop`이 아니라 `main`에서 떼고, 양쪽으로 머지합니다.

저장소를 포크하고, 브랜치를 자기 포크에 푸시한 뒤, 거기서 pull request를 엽니다. 기여하려면 쓰기 권한이
필요하지 않고, 포크에서 온 pull request는 저장소 시크릿 없이 CI를 돕니다.

## 매일 하는 일

```bash
git switch develop && git pull
git switch -c feat/short-description

cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test --workspace

# open a pull request into develop
```

커맨드는 `crates/redrob-core`의 타입이 있는 커맨드 시스템을 거칩니다. 문서를 직접 변경하는 QML 표면은
이 아키텍처가 막으려고 존재하는 바로 그것입니다. history를 우회하므로 undo가 조용히 그 변경을 다루지
않게 됩니다.

### 커밋

제목은 명령형으로 씁니다. 본문에는 _왜_를 적고, 변경이 상류 기준을 따른 것이면 어느 버전을 기준으로
확인했는지 밝힙니다.

## CI가 검사하는 것

`.github/workflows/verify-upstream.yml`은 모든 pull request와 `main`, `develop`으로의 푸시에서 돕니다.
`permissions: contents: read`를 선언하고 시크릿을 쓰지 않으므로, 포크에서 온 pull request도 이 저장소의
브랜치에서 온 것과 같은 실행을 받습니다. 잡은 셋이고, 모두 머지 전에 필수입니다.

| 검사 | 무엇을 단정하는가 |
| --- | --- |
| **상류 핀이 형식에 맞고 도달 가능한가** | `docs/upstream-sources.toml`의 핀이 파싱되고, 상류에서 해석되며, `upstream/`은 여전히 커밋될 수 없다 |
| **Rust 워크스페이스** | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace --locked` |
| **배포 산출물이 최신인가** | `tools/generate_distribution_artifacts.py --check` — 생성된 `THIRD_PARTY_NOTICES.md`와 `SOURCE_OFFER.md`가 트리와 일치한다 |

핀 잡은 "Krita를 따른다"를 주장이 아니라 검사 가능한 진술로 유지해 주는 것입니다. 이 잡은 의도적으로
753 MB의 상류 트리를 **내려받지 않습니다**.

배포 잡은 정돈 게이트가 아니라 라이선스 게이트입니다. 이 프로젝트는 GPL-3.0-or-later이고 모든 설치본이
고지와 대응 소스 아카이브를 함께 배포하므로, 그 둘이 낡아 있으면 컴플라이언스 결함입니다. 의존성,
아카이브 이름, `LICENSE`, `COPYRIGHT`를 바꾸면
`python3 tools/generate_distribution_artifacts.py --generate --gegl OFF --krita OFF`를 다시 돌리고 결과를
커밋합니다. 생성기는 의도적으로 `--locked --offline`으로 Cargo 메타데이터를 읽습니다 — 그 오프라인성은
고지 자체에 단정되어 있습니다 — 그러므로 캐시가 비어 있으면 먼저 `cargo fetch --locked`를 돌립니다.

## 의존성 업데이트

Dependabot **버전 업데이트는 꺼져 있습니다**. 상시로 pull request 대기열을 만들고 각각 CI를 한 번씩
돌려야 했는데, 이 저장소에서는 그 비용이 잡아내는 것보다 컸습니다. 두 가지가 그 자리를 대신합니다.

- **Dependabot 보안 업데이트는 켜져 있습니다.** 알려진 취약점이 있는 의존성에 대해서는 여전히
  pull request가 자동으로 열립니다. 사람을 방해할 가치가 있는 쪽은 이것입니다.
- **advisory 잡이 모든 pull request를 막습니다.** `cargo audit`이 커밋된 lockfile을 검사하므로,
  아무도 알림을 읽지 않아도 취약한 의존성은 머지되지 않습니다. 보안 업데이트는 알림이고, 이것이 통제입니다.

그래서 일상적인 버전 올리기는 의도적으로 합니다. 지금 하는 변경에 필요한 것만 같은 pull request에서
올리고, 이유를 본문에 적으세요. 관련 없는 버전 변경을 기능 브랜치에 쓸어 담지 않습니다.
