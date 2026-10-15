# 기여하기

[English](./CONTRIBUTING.md) · **한국어**

도와주셔서 감사합니다. 이 문서는 이 저장소의 작업 규약입니다: 브랜치 이름, 머지 전에 초록이어야 하는 것,
그리고 이 저장소를 특별하게 만드는 한 가지. 조직 공통 규약인
[redrob-labs/.github CONTRIBUTING.md](https://github.com/redrob-labs/.github/blob/main/CONTRIBUTING.md)와
[FORKS.md](https://github.com/redrob-labs/.github/blob/main/FORKS.md)도 함께 적용되며, 둘이 다를 때는
자기 빌드를 아는 이 문서가 우선합니다.

## 이것은 무엇이고, 무엇이 아닌가

레드롭 캔버스는 **세 상류 편집기를 감싸는 셸이고, 그 어느 것의 포크도 아닙니다.** PhotoCraft, VectorCraft,
DesignCraft(그리고 공유 craft-libs)는 `crafts/` 아래 git subtree로 통째로 들어와 Photo, Vector, Layout
모드로 돕니다. 레드롭의 코드는 셸(`apps/redrob-canvas`), 에이전트(`crates/redrob-agent`), AI 도구
층(`crates/redrob-ai`)입니다.

책임의 경계를 분명히 해 두어야 합니다.

- **편집기의 동작은 그 상류의 것입니다.** PSD가 잘못 열리면 PhotoCraft의 버그이니
  [storytold/photocraft](https://github.com/storytold/photocraft)에서 고치고, 그 수정은
  `scripts/fetch-upstream.sh`로 여기에 돌아옵니다. `crafts/` 아래 파일을 직접 고치지 마세요. 다음 pull에서
  충돌이 되고, 다음 squash에서 사라집니다.
- **셸, 에이전트, AI 층, 설치 프로그램, 그리고 이 저장소의 절차는 우리 것**이며, 그곳의 풀 리퀘스트는
  상류에 아무것도 빚지지 않습니다.

그러므로 편집기 동작을 바꾸는 풀 리퀘스트는 두 가지입니다: 상류의 변경, 그리고 여기의 핀 올리기.

## 브랜치 모델

오래 사는 브랜치는 둘입니다. `develop`에 작업이 모이고, `main`은 릴리스된 상태입니다.

- **`develop`**은 기본 브랜치이자 통합 브랜치입니다. 작업 브랜치를 여기서 잘라 다시 여기로 풀 리퀘스트를
  엽니다.
- **`main`**은 릴리스된 상태입니다. `release/*`와 `hotfix/*` 브랜치의 풀 리퀘스트만 받고, 릴리스 태그는
  여기서 찍습니다.
- **작업 브랜치**는 `develop`에서 자른 `<type>/<short-slug>`입니다. 예: `fix/mode-switch-focus`,
  `feat/ai-panel`. 타입 목록은 조직 공통의 한 목록이며 공유 `gitflow.yml`이 검사합니다:
  `feat fix chore docs test refactor perf ci build sync hotfix release`. 브랜치 이름은 변경이
  무엇인지를 말하고 무엇이 만들었는지는 말하지 않으므로, 도구나 에이전트 접두사는 실패합니다.
- 작업 브랜치의 **머지는 squash만** 쓰고 머지 시 브랜치를 지웁니다. **상류 동기화
  (`sync/upstream-<name>-<sha7>`)와 `main` 승격은 merge commit**입니다. 동기화를 squash하면 subtree의
  merge base가 사라져 다음 pull이 이미 가져온 작업을 다시 재생합니다.

두 브랜치는 GitHub 룰셋이 보호합니다: 직접 푸시 금지, force push 금지, 필수 체크, 리뷰 스레드 해결.
릴리스 워크플로는 `origin/main`에서 도달할 수 없는 커밋의 태그를 빌드하지 않으므로, `develop`에서 바로
태그를 찍으면 배포되는 대신 실패합니다.

### 릴리스

```bash
git switch develop && git pull
git switch -c release/v0.7.0
# tools/merge_workspaces.py에서 버전을 올리고(Cargo.toml을 씀) 실행, CHANGELOG.md 갱신
# main으로 풀 리퀘스트를 열어 merge commit으로 머지한 뒤 main에 태그:
git switch main && git pull
git tag -a v0.7.0 -m "Redrob Canvas v0.7.0"
git push origin v0.7.0
gh workflow run release.yml --ref main -f tag=v0.7.0
```

핫픽스는 `develop` 대신 `main`에서 자른 `hotfix/*`로 같은 모양이며 양쪽에 머지합니다. `main`을 `develop`에
되돌리는 백머지는 공유 gitflow 잡이 엽니다.

저장소를 포크하고 브랜치를 포크에 푸시한 뒤 거기서 풀 리퀘스트를 여세요. 기여에 쓰기 권한은 필요 없고,
포크의 풀 리퀘스트도 저장소 시크릿 없이 CI를 돕니다.

## 일상 작업

```bash
export PATH="$HOME/.cargo/bin:$PATH"        # rust-toolchain.toml이 1.95.0을 고릅니다
git switch develop && git pull
git switch -c feat/short-description

scripts/verify-upstream.sh
cargo fmt --all
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 tools/check_design_system.py
python3 -m unittest discover -s tools/tests -p "test_*.py"

# develop으로 풀 리퀘스트
```

루트 `Cargo.toml`은 `tools/merge_workspaces.py`가 생성합니다. 스크립트를 고치고 실행해 둘을 함께 커밋하세요.
`Cargo.lock`이 움직였으면 `cargo fetch --locked` 뒤 `python3 tools/generate_distribution_artifacts.py --generate`를
돌려 `THIRD_PARTY_NOTICES.md`를 커밋합니다.

셸은 vendoring된 디자인 시스템 토큰(`third_party/redrob-ui/`)으로 그립니다. `apps/redrob-canvas/src`의 hex 색
리터럴은 문서 색으로 표시하지 않으면 가드에서 실패합니다.

### 커밋

제목은 명령형, 소문자, 관례적 접두사. 본문에는 _왜_를 씁니다. 어디에도 em dash를 쓰지 않습니다.

## CI가 검사하는 것

`.github/workflows/verify-upstream.yml`은 모든 풀 리퀘스트와 `main`, `develop` 푸시에 돕니다.
`permissions: contents: read`를 선언하고 시크릿을 쓰지 않으므로, 포크의 풀 리퀘스트도 같은 검사를 받습니다.

| 체크 | 확인하는 것 |
| --- | --- |
| **Upstream pins are well-formed and reachable** | `upstream-base.json`과 `docs/upstream-sources.toml`이 서로, subtree squash 커밋과, GitHub과 일치; 모든 `code` 핀이 허용적 라이선스; 귀속 문자열이 온전함; `crafts/` 밖에서 ArtCraft 브랜드 자산을 건드리지 않음 |
| **Rust workspace** | `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test --workspace --locked`, 셸의 릴리스 빌드 링크 |
| **Distribution artifacts are current** | 생성된 `THIRD_PARTY_NOTICES.md`가 `Cargo.lock`과 일치; 디자인 시스템 가드; 도구 테스트 |
| **Rust security advisories** | 커밋된 락파일에 대한 `cargo audit` |

배포 잡은 귀속 게이트입니다. 모든 아카이브가 `LICENSE-MIT`, `LICENSE-APACHE`, `COPYRIGHT`, `NOTICE`,
`THIRD_PARTY_NOTICES.md`를 담으므로, 오래된 고지 파일은 받는 이에게 한 약속의 결함입니다. 생성기는 의도적으로
`--locked --offline`으로 Cargo 메타데이터를 읽으니, 캐시가 비어 있으면 먼저 `cargo fetch --locked`를 돌리세요.

## 의존성 갱신

Dependabot **버전 갱신은 꺼져 있습니다**. 풀 리퀘스트 대기열이 계속 쌓이고 각각 CI를 돌려야 해서, 이 저장소에서는
잡아내는 것보다 비용이 컸습니다. 두 가지가 그 자리를 대신합니다.

- **Dependabot 보안 갱신은 켜져 있습니다.** 알려진 취약점이 있는 의존성은 여전히 자동으로 풀 리퀘스트가 열립니다.
- **보안 공지 잡이 모든 풀 리퀘스트를 막습니다.** 커밋된 락파일에 `cargo audit`을 돌리므로, 누가 경고를 읽지
  않았더라도 취약한 의존성은 머지되지 않습니다.

대부분의 의존성은 네 subtree의 매니페스트를 통해 들어오고 subtree를 pull할 때 움직입니다. 우리 자신의 버전 올리기는
의도적으로 합니다. 지금 하는 변경에 필요한 것만, 같은 풀 리퀘스트에서, 본문에 이유를 적어서.

## 기여물의 라이선스

기여하면 그 작업이 `LICENSE-MIT`와 `LICENSE-APACHE`의 라이선스, 즉 MIT OR Apache-2.0으로 공개되는 데 동의하는
것입니다. CLA도 sign-off 요구도 없습니다. 상류의 저작권 고지는 절대 바꾸지 않습니다. 우리 고지는 그 옆에 둡니다.
