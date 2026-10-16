# Redrob Canvas 0.8.1 간격 표 -- Photoshop · GIMP 3.2.6 · Krita 5.3.4

2026-10-16. 측정 산출물은 `docs/gap/` 아래에 있다: 작업 정의 `tasks.toml`, 판정 `results/*-verdicts.toml`, PSD 왕복 `results/psd-*.{toml,md}`, GPU·AI `results/gpu-*.toml`.

## 읽는 법

| 기호 | 뜻 |
|---|---|
| O | 된다 (통과 기준 충족) |
| ~ | 느리다 (된다, 기준 시간 초과) |
| X | 깨진다 (기능은 있는데 결과가 틀리거나 끝을 못 냄) |
| - | 없다 |

Photoshop 열은 이 50개 작업의 정의 자체가 Photoshop 작업이라 전부 O 로 둔다(공식 문서 기준, 실행 측정 아님). Canvas 는 릴리즈 바이너리 0.8.1 을 `:0` 에서 사람이 하듯 조작해 판정했다. GIMP·Krita 는 배치 스크립트(python-fu / kritarunner)로 같은 작업을 돌렸다; `-` 는 그 앱에 내장 기능이 없다는 뜻이고, 사용자 조작 품질(손맛)은 이 표가 재지 않는다.

## 요약

| | Canvas 0.8.1 | Photoshop | GIMP 3.2.6 | Krita 5.3.4 |
|---|---:|---:|---:|---:|
| 된다 | 41 | 50 | 27 | 28 |
| 느리다 | 0 | 0 | 1 | 3 |
| 깨진다 | 8 | 0 | 10 | 9 |
| 없다 | 1 | 0 | 12 | 10 |

## 50개 작업

| ID | 영역 | 작업 | Canvas | PS | GIMP | Krita | Canvas 근거 |
|---|---|---|:-:|:-:|:-:|:-:|---|
| BR-01 | 브러시 | 기본 원형 브러시로 필압 스트로크 긋기 | O | O | O | O | New layer via Ctrl+Shift+N dialog; Brush (B) S-stroke drew ~43k doc px on Layer 1 (22140 c |
| BR-02 | 브러시 | 스트로크 스무딩(안정화) 켜고 떨림 없는 곡선 긋기 | O | O | O | O | Options-bar Smoothing field accepted 60 then 0; same zigzag-circle input: edge/area roughn |
| BR-03 | 브러시 | ABR 브러시 프리셋 파일 가져오기 | O | O | X | O | Brush picker gear > Import Brushes… opened portal dialog; special/src.abr (6 presets) -> s |
| BR-04 | 브러시 | 클론 스탬프로 피부 잡티 제거 | O | O | O | O | Clone Stamp (S) bar Opacity/Flow/Aligned/Sample; brush Size 300 via preset popup. Plain Al |
| BR-05 | 브러시 | 스머지(믹서) 브러시로 색 섞기 | O | O | O | O | Smudge Tool (palette; no key) Strength 70->shown 78%; black rect edge smudged into white:  |
| BR-06 | 브러시 | 대칭 페인팅(세로축 미러) | O | O | O | O | Brush options-bar symmetry icon > Vertical enabled; 2 strokes on left half only; change-ma |
| SEL-01 | 선택 | 사각 선택 + 페더 후 채우기 | O | O | O | O | Marquee (M) drag 1000x600 doc rect on 8k; Select>Modify>Feather dialog (palette) Radius fi |
| SEL-02 | 선택 | 마술봉으로 단색 배경 선택(허용치 32, 인접) | O | O | O | O | Magic Wand (W) Tolerance 32 Contiguous; one click on white bg, Select>Inverse Selection, F |
| SEL-03 | 선택 | 퀵 선택으로 피사체 선택 | O | O | - | - | Quick Selection Tool (palette; bar: Sample All Layers, Enhance Edge, 'Alt to subtract', Se |
| SEL-04 | 선택 | Color Range 로 하늘(파란색 계열) 선택 | O | O | O | O | Select>Color Range dialog (Sampled Colors, Localized Color Clusters, Fuzziness, Clear Samp |
| SEL-05 | 선택 | 자석 올가미로 제품 외곽 따기 | O | O | O | O | Magnetic Lasso Tool (palette), Width 10/Contrast 10/Freq 57 defaults; 90-point mouse trace |
| SEL-06 | 선택 | Select and Mask 로 머리카락 가장자리 다듬기 | O | O | - | - | Select>Select and Mask (palette 'Select and Mask') on the Select Subject selection opens a |
| TR-01 | 변형 | Free Transform 으로 레이어 축소·회전 | O | O | O | O | Ctrl+T on Background refused (status: 'not available in the current state'); after Layer F |
| TR-02 | 변형 | Warp(Arc 프리셋 → 커스텀 메시) 변형 | O | O | X | X | Warp (palette) options bar Grid/Warp/Bend; Warp=Arc Bend 30: bottom edge of the full-canva |
| TR-03 | 변형 | Perspective Crop 으로 기울어진 사각 피사체 펴기 | O | O | O | O | Perspective Crop (palette) options bar Width/Height/Resolution/Front Image/Show Grid; 4 co |
| TR-04 | 변형 | Liquify 로 얼굴 윤곽 밀기 | O | O | O | O | Liquify (palette) opens in-window panel (Brush Tool Options Size/Density/Pressure/Rate, Re |
| LY-01 | 레이어 | 새 레이어 + 블렌드 모드 Multiply + 불투명도 60% | O | O | O | O | Run on the open landscape-8k doc (same math as product-1k): new layer filled 50% Gray, Lay |
| LY-02 | 레이어 | 레이어 3개 그룹화 후 Merge Visible | O | O | O | O | layers-styles.psd (11 type layers with fx): Shift+click selected Satin, Stroke, Pattern Ov |
| LY-03 | 레이어 | 레이어 스타일: Drop Shadow + Stroke 추가 | O | O | O | O | Product cut to Layer 1 (Quick Selection > Select Subject, Layer Via Cut; hidden Background |
| LY-04 | 레이어 | 클리핑 마스크로 사진을 글자에 가두기 | O | O | - | O | landscape-8k: Layer From Background, Horizontal Type Tool (palette 'Type Tool' - 'Horizont |
| LY-05 | 레이어 | 조정 레이어(Curves)를 비파괴로 추가하고 다시 편집 | O | O | O | O | Palette 'Curves (Layer > New Adjustment Layer)' opens a modal Curves dialog whose OK appli |
| MK-01 | 마스크 | 선택에서 레이어 마스크 만들고 브러시로 수정 | O | O | O | O | Select Subject on the 4k portrait, Layer From Background, Layer>Layer Mask>Reveal Selectio |
| MK-02 | 마스크 | 마스크 Density/Feather 비파괴 조절 | O | O | - | - | With the MK-01 mask thumb selected the Properties panel shows 'Layer Mask' with Density (% |
| MK-03 | 마스크 | 채널을 선택으로 불러와 알파 마스크 만들기 | O | O | X | X | Channels panel (RGB/Red/Green/Blue, Ctrl+2..5): plain xdotool Ctrl+click on the Blue thumb |
| AD-01 | 조정 | Levels 로 블랙/화이트 포인트 잡기 | O | O | X | O | Levels dialog (palette 'Levels'; Channel RGB, histogram with 3 input handles, Input fields |
| AD-02 | 조정 | Curves 로 S커브 대비 올리기 | O | O | O | O | Curves dialog (palette 'Curves'; Eyedroppers, Channel dropdown RGB/Red/Green/Blue, 4x4 gri |
| AD-03 | 조정 | Hue/Saturation 으로 특정 색(Reds)만 바꾸기 | O | O | O | X | product-1k.png has NO red or blue pixels at all (0 px with S>0.12 in hue 345-15 or 200-260 |
| AD-04 | 조정 | Shadows/Highlights 로 역광 보정 | X | O | O | - | Image>Adjustments>Shadows/Highlights dialog (palette 'Shadows') with Shadow/Highlight Amou |
| AD-05 | 조정 | RAW(DNG) 열어 Camera Raw 에서 노출·화이트밸런스 보정 | X | O | - | X | File>Open on raw-sample.dng (Pentax K-x 12-bit) opens a separate 'Camera Raw (raw-sample.d |
| FL-01 | 필터 | Gaussian Blur 20px 를 8k 전체에 적용 | O | O | O | ~ | Filter>Blur>Gaussian Blur dialog (Radius field+slider, Preview, zoom -/+, OK/Cancel); Radi |
| FL-02 | 필터 | Unsharp Mask 로 선명도 올리기 | X | O | O | X | Filter>Sharpen>Unsharp Mask dialog (palette 'Unsharp Mask'): Amount %, Radius px, Threshol |
| FL-03 | 필터 | Content-Aware Fill 로 배경 물체 지우기 | O | O | - | O | Edit>Content-Aware Fill dialog (Sampling Auto/Margin/Channel, Color Adaptation, Rotation,  |
| FL-04 | 필터 | Lens Correction 으로 배럴 왜곡·비네팅 자동 보정 | X | O | X | - | Filter>Lens Correction dialog (palette) on the opened DNG (EXIF: PENTAX K-x, smc PENTAX-DA |
| FL-05 | 필터 | 스마트 오브젝트에 Smart Filter(High Pass) 비파괴 적용 | O | O | O | O | Layer>Smart Objects>Convert to Smart Object (palette) on the portrait (title 'Layer 0'), F |
| TX-01 | 텍스트 | 포인트 텍스트 입력 후 폰트·크기·색 바꾸기 | X | O | O | O | Horizontal Type Tool on product-1k at 100%: options bar has font (Inter) / style (Regular) |
| TX-02 | 텍스트 | 단락 텍스트 상자에 양쪽 정렬 긴 문장 흘리기 | X | O | X | X | Type tool drag on landscape-8k at 100% made an 800x400 paragraph text box (Properties: typ |
| TX-03 | 텍스트 | Warp Text(Arc) 적용 후 편집 가능 유지 | O | O | - | O | Type>Warp Text dialog (palette) on the TX-01-style 72 pt 'Redrob Canvas' layer: Style drop |
| PS-01 | PSD | 레이어 스타일 PSD 열어 합성 일치 확인 | O | O | X | X | layers-styles.psd (psd-tools layer_effects fixture, 800x640) opens at 100% with a warning  |
| PS-02 | PSD | 스마트 오브젝트 PSD 열고 Edit Contents 로 수정 반영 | O | O | - | - | smart-object.psd (ag-psd fixture, 300x300, placed layers 'picture' 214x238@44,37 and 'kitt |
| PS-03 | PSD | 조정 레이어 PSD 의 조정값 보존·재편집 | O | O | X | X | adjustment-layers.psd (psd-tools fill_adjustments, 256x256, 16 adjustment layers) opens wi |
| PS-04 | PSD | 텍스트 레이어 PSD 를 편집 가능 상태로 열기 | O | O | X | O | text-layers.psd (ag-psd text-test, 200x200, type layers 'a' with fx and 'b') opens at 100% |
| PS-05 | PSD | 8k 대형 PSD 열기·저장 왕복 시간과 손실 확인 | O | O | O | ~ | large-8k.psd (7680x4320, 5 layers, 303 MB): File>Open -> document title 2617 ms after Ente |
| EX-01 | 내보내기 | Export As 로 JPEG 품질 80 내보내기 | O | O | O | O | On the flat Background-only portrait, File>Export>Export As skipped the settings dialog an |
| EX-02 | 내보내기 | WebP 로 50% 리사이즈 내보내기(투명 유지) | X | O | O | O | product-1k: Layer From Background, Select Subject + Inverse Selection + Edit>Clear left th |
| EX-03 | 내보내기 | 8k 문서 PNG 빠른 내보내기 시간 | O | O | ~ | ~ | Layer 1 filled 50% Gray (Edit>Fill), Layers panel blend dropdown -> Multiply (view centre  |
| VC-01 | 벡터 | SVG 아이콘 열어 펜 도구로 앵커 이동 후 SVG 저장 | X | O | X | X | Ctrl+2 switches to Vector mode (menu File/Edit/Object/Type/Select/Effect/View/Window/Help, |
| VC-02 | 벡터 | Pathfinder Unite 로 겹친 도형 합치기 | O | O | - | - | Vector mode: svg/illustration-complex.svg (1486x1646, 4092 <path> + 1381 <circle> + 214 <e |
| LO-01 | 레이아웃 | A3 포스터 IDML 열어 PDF/X 로 내보내기 | O | O | - | - | Ctrl+3 switches to Layout mode (File/Edit/Layout/Type/Object/Table/View/Window/Help). No I |
| AI-01 | AI | 원클릭 배경 제거(피사체 분리) | O | O | - | - | Select>Subject (Quick Selection bar button) on the 4k portrait: marching ants appeared wit |
| AI-02 | AI | 프롬프트 기반 생성형 채우기(Generative Fill) | - | O | - | - | No Generative Fill: command palette has no match for 'Generative' (empty list), 'Generate' |

## 시간 (ms, timed 작업)

| ID | 작업 | Canvas | GIMP | Krita |
|---|---|---:|---:|---:|
| BR-01 | 기본 원형 브러시로 필압 스트로크 긋기 | 300 | 23 | 0 |
| SEL-03 | 퀵 선택으로 피사체 선택 | 502 | 533 | - |
| SEL-04 | Color Range 로 하늘(파란색 계열) 선택 | 834 | 599 | - |
| SEL-06 | Select and Mask 로 머리카락 가장자리 다듬기 | 3025 | - | - |
| TR-04 | Liquify 로 얼굴 윤곽 밀기 | 1 | - | - |
| AD-04 | Shadows/Highlights 로 역광 보정 | 700 | 1833 | - |
| AD-05 | RAW(DNG) 열어 Camera Raw 에서 노출·화이트밸런스 보정 | 1943 | 51 | - |
| FL-01 | Gaussian Blur 20px 를 8k 전체에 적용 | 1779 | 1517 | 26945 |
| FL-03 | Content-Aware Fill 로 배경 물체 지우기 | 162 | - | - |
| PS-05 | 8k 대형 PSD 열기·저장 왕복 시간과 손실 확인 | 2617 | 19080 | 23966 |
| EX-03 | 8k 문서 PNG 빠른 내보내기 시간 | 2822 | 107824 | 12267 |
| AI-01 | 원클릭 배경 제거(피사체 분리) | 2449 | - | - |
| AI-02 | 프롬프트 기반 생성형 채우기(Generative Fill) | 0 | - | - |

## Canvas 가 뒤지는 자리

경쟁 앱 중 하나라도 더 잘하는 작업:

| ID | 작업 | Canvas | GIMP | Krita | 무엇이 문제인가 |
|---|---|:-:|:-:|:-:|---|
| AD-04 | Shadows/Highlights 로 역광 보정 | X | O | - | Image>Adjustments>Shadows/Highlights dialog (palette 'Shadows') with Shadow/Highlight Amount-Tone-Radius, Color, Midtone, Black/White Clip,  |
| FL-02 | Unsharp Mask 로 선명도 올리기 | X | O | X | Filter>Sharpen>Unsharp Mask dialog (palette 'Unsharp Mask'): Amount %, Radius px, Threshold fields + sliders, Preview; no OK/Cancel buttons  |
| TX-01 | 포인트 텍스트 입력 후 폰트·크기·색 바꾸기 | X | O | O | Horizontal Type Tool on product-1k at 100%: options bar has font (Inter) / style (Regular) / size / anti-alias / align / colour swatch (defa |
| EX-02 | WebP 로 50% 리사이즈 내보내기(투명 유지) | X | O | O | product-1k: Layer From Background, Select Subject + Inverse Selection + Edit>Clear left the shoe on a transparent canvas (Magic Wand click m |

Canvas 에서 깨진 나머지(경쟁 앱도 못 하는 것): AI-02, AD-05, FL-04, TX-02, VC-01

## PSD 충실도 (105개 파일, `results/psd-summary.md`)

| 비교 | 같음 (<0.5% RMSE) | 조금 다름 (0.5~5%) | 깨짐 (>=5%) |
|---|---:|---:|---:|
| Canvas 저장 전 vs 저장 후 (코덱 자기 일관성) | 105 | 0 | 0 |
| Canvas 렌더 vs GIMP 렌더 (같은 원본) | 53 | 6 | 46 |
| GIMP 가 Canvas 저장 파일을 읽은 결과 vs 원본 | 77 | 2 | 26 |

레이어·그룹·마스크·텍스트·조정 레이어·스마트 오브젝트 수는 전부 보존된다. 열기 중앙값 28 ms, 저장 26 ms, 8K 303 MB 파일 열기 1.3 s / 저장 2.3 s.

문제는 렌더다. 포토샵 파일 105개 중 46개는 다른 앱과 다르게 보인다. 원인은 텍스트 셰이핑, 조정 레이어 계산, 혼합 모드 해석, CMYK/Lab 변환. 포토샵 사용자가 자기 파일을 열었을 때 "다르게 보인다"는 첫인상이 여기서 나온다.

## GPU · AI (gui-box-gpu, NVIDIA L40S, `results/gpu-*.toml`)

| 측정 | 결과 |
|---|---|
| 시작 (따뜻한 상태 / 처음) | 0.26~0.43 s / 1.39 s |
| 4K JPEG / 8K JPEG / 8K PSD 303 MB 열기 | 1.12 s / 2.14 s / 3.02 s |
| 모드 전환 Photo->Vector->Layout->Photo | 21 / 12 / 2 ms |
| 가우시안 블러 r=20, 4K / 8K | 668 ms / 1826 ms |
| Levels·Curves 적용 4K / 8K | ~250~300 ms / ~700 ms |
| 브러시 프레임 p50, 1 레이어 4K / 10 레이어 8K | 4.5 ms (~220 fps) / 6.3 ms (~160 fps) |
| VRAM 1 -> 10 레이어, 4K / 8K | 1.06 -> 1.83 GB / 1.35 -> 3.65 GB |

AI 도크 엔진 (IOPaint 1.6 + rembg, `crates/redrob-ai` 가 쓰는 것과 동일):

| 작업 | 계산 시간 | 왕복 시간 | 비고 |
|---|---:|---:|---|
| 인페인트 lama, 마스크 5%, 1K / 4K / 8K | 0.32 / 0.67 / 1.06 s | 0.65 / 7.7 / 22 s | 그림 전체를 base64 PNG 로 두 번 보내서 느림 |
| 아웃페인트 25%, 1K | 1.3 s | | 된다 |
| 아웃페인트 25%, 4K / 8K (Crop 전략, 기본값) | 실패 | | CUDA 메모리 초과 (36 / 98 GiB 요구) |
| 아웃페인트 4K / 8K (Resize 전략) | 1.8 / 4.0 s | | 되지만 이음새 보임 (7.5 vs 기준 1.0) |
| 배경 제거 u2net / isnet | 0.25 s | 3.6~7.6 s (4K/8K) | 왕복이 전송 시간 |
| 업스케일 realesrgan x4, 1K | `results/gpu-ai.toml` 참조 | | |

패키징 결함: Linux tar.gz 바이너리가 `libxkbcommon-x11.so.0` 없는 호스트에서 시작 시 죽는다(.deb 의존성에도 없음).

## 어디서 빈약해 보이는가

기능 개수로는 Canvas 가 GIMP·Krita 보다 앞선다(41 vs 27 vs 28). "포토샵보다 빈약하다"는 평은 개수가 아니라 다음 네 곳에서 나온다.

1. 포토샵 파일을 열면 절반 가까이가 다르게 보인다 (PSD 렌더 46/105).
2. 다이얼로그가 끝을 못 낸다. Unsharp Mask, Shadows/Highlights 에 OK/적용 버튼이 없다 (FL-02, AD-04). 텍스트 도구는 옵션을 바꿔도 글자에 반영이 안 된다 (TX-01, TX-02).
3. AI 가 느리거나 터진다. 인페인트 4K 왕복 7.7 s, 8K 22 s. 아웃페인트는 4K 부터 실패. 생성 채우기(프롬프트)는 없다 (AI-02).
4. 벡터 모드는 복잡한 SVG 를 못 연다 (VC-01: 심볼 전용 SVG 가 0 패스). 내보내기 크기가 레이어 범위로 잘린다 (EX-02).

## 다음 3개 릴리즈 제안

| 릴리즈 | 목표 | 들어갈 것 | 통과 기준 |
|---|---|---|---|
| v0.9.0 "끝까지 되는 도구" | 깨진 8개를 0으로 | FL-02·AD-04 다이얼로그 OK/취소, TX-01·TX-02 텍스트 옵션 반영, EX-02 내보내기 캔버스 크기·리사이즈, VC-01 `<symbol>`/`<use>` 전개, AD-05 Camera Raw 결과를 문서로 돌려보내기, FL-04 EXIF 초점거리 읽기, Linux .deb 에 libxkbcommon-x11 의존성 | 50개 표에서 X 가 0, PSD 코덱 자기 일관성 105/105 유지 |
| v0.10.0 "포토샵 파일 그대로" | PSD 렌더 46 -> 10 이하 | 텍스트 레이어 셰이핑(폰트 폴백·커닝), 조정 레이어 수식 Photoshop 과 대조(Levels·Curves·HSL·Color Lookup), 혼합 모드 16종 픽셀 대조, CMYK/Lab ICC 변환, GIMP 읽기 호환 26 -> 5 이하 | `psd-summary.md` vs GIMP 밴드 broken <= 10, 읽기 호환 broken <= 5 |
| v0.11.0 "로컬 AI 가 차별점" | AI 왕복을 계산 시간에 붙이고 생성 채우기 추가 | 엔진과 이미지를 공유 메모리·파일 경로로 주고받기(base64 제거, 목표 4K 왕복 < 1.5 s), 아웃페인트 타일링(4K·8K 메모리 초과 제거), 생성 채우기(프롬프트, SD-inpaint 또는 FLUX-fill 로컬), 피사체 선택 품질 isnet 기본 | 인페인트 4K 왕복 < 1.5 s, 아웃페인트 8K 성공, AI-02 O |

4번째 이후: 브러시 엔진 깊이(압력 곡선·텍스처·안정화), 액션/배치, ABR·ATN 불러오기, 플러그인(MCP) -- 이번 표에서는 전부 O 라 우선순위가 뒤다.

## 측정 환경과 한계

- Canvas: 릴리즈 0.8.1 Linux x86_64 바이너리, 이 호스트(32코어, GPU 없음) 메인 디스플레이 `:0` 에서 xdotool 로 조작, 판정은 스크린샷 픽셀 측정. 사람의 손맛(브러시 지연·안티앨리어싱 품질)은 재지 않았다.
- GIMP 3.2.6 · Krita 5.3.4: 사용자 홈 설치, python-fu / kritarunner 배치. GUI 전용 기능은 문서·메뉴 존재 여부로 판정.
- Photoshop: 이 호스트에 없다. 50개 작업 정의가 Photoshop 작업이므로 열은 참조값이다.
- GPU: gui-box-gpu (L40S 46 GB, NICE DCV 실제 X 세션). 이 호스트와의 CPU 비교는 `:0` 이 Canvas 테스트에 쓰이고 있어 못 했다.
- 두 번째·세 번째 Canvas 실행 에이전트가 중간에 멈춰 세 번에 나눠 돌렸다. 판정 파일은 이어 쓰기라 결과는 하나다.
