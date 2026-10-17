# Redrob Canvas 핵심 작업 50개 (vs Photoshop / GIMP 3 / Krita 5)

Stage 1-A 산출물. 기계용 원본은 `docs/gap/tasks.toml` 이고 이 표는 같은 내용이다.
판정(verdict)은 Stage 2 가 `works | slow | broken | missing` 중 하나로 채운다.
`canvas_entry` 는 코드에 그 명령/메뉴 문자열이 있다는 뜻일 뿐, 동작한다는 뜻이 아니다. 없으면 `unknown`.

경로 접두: E = `crafts/photocraft/crates/engine/src`, U = `crafts/photocraft/crates/ui-egui/src`

## 영역별 개수

| 영역 | 개수 | timed |
|---|---:|---:|
| brush | 6 | 1 |
| selection | 6 | 3 |
| transform | 4 | 1 |
| layer | 5 | 0 |
| mask | 3 | 0 |
| adjust | 5 | 2 |
| filter | 5 | 2 |
| text | 3 | 0 |
| psd | 5 | 1 |
| export | 3 | 1 |
| vector | 2 | 0 |
| layout | 1 | 0 |
| ai | 2 | 2 |
| **합계** | **50** | **13** |

## brush (6)

### BR-01 — 기본 원형 브러시로 필압 스트로크 긋기

- input: `photo/product-1k.png`  · mode: photo  · timed: yes
- steps:
  1. 파일을 열고 새 빈 레이어를 추가한다
  2. 브러시 도구(B), 크기 40px, 경도 50%, 불투명도 100%로 설정한다
  3. 태블릿(없으면 마우스)으로 좌상단에서 우하단까지 S자 스트로크 1회
  4. 필압이 있으면 스트로크 폭이 압력에 따라 변하는지 확인한다
- pass: 새 레이어의 비투명 픽셀이 1,000개 이상 생기고, 스트로크 시작~끝 그리기 지연이 100ms 미만(화면 갱신 시각 기준)이다.
- Photoshop: Brush Tool(B), Options bar에서 Size/Hardness/Opacity, Pressure for Size 토글.
- canvas_entry: paint.stroke / paint.brush (E/brush_cmds.rs, E/paint_cmds.rs; 필압 crates/tablet/src/x11.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### BR-02 — 스트로크 스무딩(안정화) 켜고 떨림 없는 곡선 긋기

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. 새 빈 레이어 추가, 브러시 크기 12px
  2. Smoothing(안정화)을 60%로 설정한다
  3. 손떨림을 흉내내 지그재그로 흔들며 원을 1회 그린다
  4. Smoothing 0%로 같은 입력을 반복해 비교한다
- pass: Smoothing 60% 결과 스트로크의 곡률 변화(연속 세그먼트 각도 차 평균)가 0%일 때보다 50% 이상 작다.
- Photoshop: Brush Options bar의 Smoothing(0~100%) + Pulled String/Stroke Catch-up 옵션.
- canvas_entry: brush smoothing 파라미터 (E/brush_cmds.rs 'smoothing'; U/stroke_constraint.rs, U/stroke_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### BR-03 — ABR 브러시 프리셋 파일 가져오기

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. Brushes 패널 메뉴에서 Import Brushes 를 선택한다
  2. corpus 외부 샘플 .abr(기본 PS 브러시 세트 아무것)을 선택한다
  3. 가져온 프리셋 그룹을 펼쳐 프리셋 하나를 선택해 스트로크를 긋는다
- pass: .abr 안의 프리셋 개수와 패널에 추가된 프리셋 개수가 같고, 선택한 프리셋 스트로크가 기본 원형 브러시와 다른 모양(픽셀 마스크 비교 차이 >5%)이다.
- Photoshop: Brushes 패널 ≡ › Import Brushes… (.abr).
- canvas_entry: preset_import (E/preset_import_cmds.rs, E/preset_store.rs; 파서 crates/psd/src/abr.rs; U/brushes_tab.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### BR-04 — 클론 스탬프로 피부 잡티 제거

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. Clone Stamp 도구(S), 크기 60px, Aligned 켬
  2. Alt+클릭으로 잡티 옆 깨끗한 피부를 소스로 지정한다
  3. 잡티 위를 2~3회 짧게 칭한다
  4. 100% 줌으로 경계가 보이는지 확인한다
- pass: 잡티 영역(64x64 crop) 색 표준편차가 처리 전 대비 40% 이상 줄고, 소스 지정 Alt+클릭이 동작한다.
- Photoshop: Clone Stamp Tool(S), Alt+click source, Options bar Aligned/Sample.
- canvas_entry: clone / clone.preview (E/stamp_cmds.rs, E/retouch_cmds.rs 'aligned'; U/stamp_tests.rs, U/alt_grab.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### BR-05 — 스머지(믹서) 브러시로 색 섞기

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. 새 레이어에 빨강/파랑 두 사각형을 인접하게 채운다
  2. Smudge 도구, Strength 70%, 크기 50px
  3. 경계를 가로질러 좌→우로 3회 드래그한다
- pass: 경계 주변 20px 띠에서 중간색(보라 계열, R>80 && B>80) 픽셀이 전체의 30% 이상 생긴다.
- Photoshop: Smudge Tool(Blur 그룹), Strength; Mixer Brush Tool(Wet/Load/Mix).
- canvas_entry: paint.smudge (E/retouch_cmds.rs); mix/wet 파라미터 (E/brush_cmds.rs 'mix','wet')
- verdict: `works|slow|broken|missing` (Stage 2)

### BR-06 — 대칭 페인팅(세로축 미러)

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. 브러시 Options bar의 Symmetry 메뉴에서 Vertical 을 켠다
  2. 축 위치를 캔버스 중앙에 둔다
  3. 왼쪽 절반에만 스트로크 2회를 긋는다
- pass: 오른쪽 절반을 좌우 반전한 영상과 왼쪽 절반의 픽셀 차이(RMSE)가 2/255 미만이다.
- Photoshop: Brush Options bar 나비 아이콘 › Vertical/Horizontal/Radial Symmetry.
- canvas_entry: symmetry (E/symmetry_cmds.rs 'path.set','rotation','segments'; U/symmetry_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

## selection (6)

### SEL-01 — 사각 선택 + 페더 후 채우기

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: no
- steps:
  1. Rectangular Marquee(M)로 중앙 1000x600 영역을 드래그한다
  2. Select › Modify › Feather 30px
  3. 새 레이어에서 Edit › Fill 흰색
- pass: 채운 영역 가장자리 알파가 60px 폭에서 0→255로 단조 증가하고, 중앙은 255이다.
- Photoshop: Marquee Tool(M) → Select › Modify › Feather… → Edit › Fill.
- canvas_entry: select.rect, select.modify.feather, edit.fill (E/selection_cmds.rs, E/fill_cmds.rs; U/marquee_tests.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### SEL-02 — 마술봉으로 단색 배경 선택(허용치 32, 인접)

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. Magic Wand(W), Tolerance 32, Contiguous 켬
  2. 제품 바깥 흰 배경을 1회 클릭한다
  3. Select › Inverse 로 제품만 남긴다
- pass: 선택 마스크가 배경 참값 마스크와 IoU 0.95 이상이고 제품 내부에 구멍(미선택 섬) 수가 2개 이하이다.
- Photoshop: Magic Wand Tool(W): Tolerance, Contiguous, Sample All Layers.
- canvas_entry: select.similar/select.grow + tolerance·contiguous (E/selection_cmds.rs); select.inverse (U/menus.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### SEL-03 — 퀵 선택으로 피사체 선택

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: yes
- steps:
  1. Quick Selection(W) 크기 80px
  2. 인물 몸통 위를 2~3회 드래그해 선택을 키운다
  3. Alt 드래그로 배경에 번진 부분을 뺀다
- pass: 결과 마스크가 인물 참값과 IoU 0.90 이상이고 드래그당 응답 지연이 300ms 미만이다.
- Photoshop: Quick Selection Tool(W): 드래그로 추가, Alt 드래그로 제외, Enhance Edge.
- canvas_entry: select.quick (E/smartselect_cmds.rs; U/quick_pick.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### SEL-04 — Color Range 로 하늘(파란색 계열) 선택

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: yes
- steps:
  1. Select › Color Range…
  2. Select: Sampled Colors, 하늘을 클릭, Fuzziness 60
  3. Shift+클릭으로 하늘 2곳을 더 샘플링하고 OK
- pass: 선택 마스크의 하늘 영역 IoU 0.85 이상, 다이얼로그 미리보기 갱신이 1초 미만(8k)이다.
- Photoshop: Select › Color Range…: Fuzziness, Localized Color Clusters, Range.
- canvas_entry: select.colorRange (U/menu_catalog.rs, U/color_range_ui.rs; E/selection_cmds.rs 'fuzziness','localized')
- verdict: `works|slow|broken|missing` (Stage 2)

### SEL-05 — 자석 올가미로 제품 외곽 따기

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. Magnetic Lasso(L 그룹), Width 10px, Contrast 10%, Frequency 57
  2. 제품 외곽을 따라 천천히 한 바퀴 돌아 시작점에서 닫는다
  3. Backspace 로 마지막 점 하나를 지우고 다시 이어 본다
- pass: 닫힌 선택의 외곽선이 참값 외곽과 평균 거리 4px 이내이고 Backspace 되돌리기가 동작한다.
- Photoshop: Magnetic Lasso Tool: Width/Contrast/Frequency, Delete 로 앵커 제거.
- canvas_entry: select.lasso magnetic (E/magnetic_cmds.rs 'width','contrast','trace'; U/magnetic_lasso_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### SEL-06 — Select and Mask 로 머리카락 가장자리 다듬기

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: yes
- steps:
  1. SEL-03 결과 선택을 유지한 채 Select › Select and Mask…
  2. Refine Edge Brush 로 머리카락 외곽을 1회 쓸어 준다
  3. Output: Layer Mask, Decontaminate Colors 켬, OK
- pass: 머리카락 영역(참값 알파 0.2~0.8 구간)에서 결과 알파 MAE 0.15 이하이고 레이어 마스크로 출력된다.
- Photoshop: Select › Select and Mask…: Refine Edge Brush, Decontaminate Colors, Output To.
- canvas_entry: select.selectAndMask → select.refineEdge (U/menus.rs:527, U/select_mask_view.rs; E/smartselect_cmds.rs 'decontaminate')
- verdict: `works|slow|broken|missing` (Stage 2)

## transform (4)

### TR-01 — Free Transform 으로 레이어 축소·회전

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. 레이어를 선택하고 Cmd/Ctrl+T
  2. Options bar에 W 50% H 50%, 각도 15° 입력 후 Enter
  3. Edit › Undo 로 되돌린 뒤 다시 Redo
- pass: 결과 레이어 경계 상자 폭이 원본의 50%±1px, 회전 15°±0.5° 이고 Undo/Redo 후 픽셀이 원본과 동일(해시 일치)하다.
- Photoshop: Edit › Free Transform(Cmd+T), Options bar W/H/각도, Enter 커밋.
- canvas_entry: edit.freeTransform / edit.transform.scale·rotate (U/menus.rs:39, U/transform_tool.rs; E/transform_cmds.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### TR-02 — Warp(Arc 프리셋 → 커스텀 메시) 변형

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. Edit › Transform › Warp
  2. 프리셋 Arc, Bend 30% 적용
  3. Custom 으로 바꿔 메시 제어점 하나를 80px 드래그, Enter
- pass: 결과 레이어 상단 에지가 아크 형태(중앙 y 가 양끝 y 보다 30%±5% 더 위)이고 커스텀 드래그 지점 픽셀이 이동한다.
- Photoshop: Edit › Transform › Warp: Warp 프리셋(Arc, Bend…), Custom 메시 3x3/4x4/5x5, Split.
- canvas_entry: edit.transform.warp (E/warp_cmds.rs 'arc','bend','mesh','grid'; U/warp_preview.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### TR-03 — Perspective Crop 으로 기울어진 사각 피사체 펴기

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. Perspective Crop 도구(C 그룹)
  2. 제품 포장 사각형의 네 모서리를 클릭한다
  3. Enter 로 자른다
- pass: 결과 이미지에서 포장 네 변이 축에 평행(각 변 기울기 1° 이내)하고 결과 크기가 Options bar 지정 W/H 와 같다.
- Photoshop: Perspective Crop Tool: 네 모서리 클릭 → Enter.
- canvas_entry: perspective_crop (E/perspective_crop_cmds.rs 'corners','interpolation'; U/perspective_crop_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### TR-04 — Liquify 로 얼굴 윤곽 밀기

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: yes
- steps:
  1. Filter › Liquify…
  2. Forward Warp 도구, 브러시 300px, Pressure 50
  3. 턱선 바깥→안쪽으로 3회 드래그, OK
- pass: 턱선 위치가 안쪽으로 10px 이상 이동하고(엣지 검출 비교) 4k 미리보기 갱신이 드래그당 200ms 미만이다.
- Photoshop: Filter › Liquify…: Forward Warp(W), Brush Size/Pressure/Density, Face-Aware Liquify.
- canvas_entry: filter.liquify (E/distort_cmds.rs 'filter.liquify','pressure','density'; U/liquify_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

## layer (5)

### LY-01 — 새 레이어 + 블렌드 모드 Multiply + 불투명도 60%

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. Layer › New › Layer, 50% 회색으로 채운다
  2. Layers 패널에서 Blend Mode: Multiply, Opacity 60%
  3. 합성 결과를 PNG로 내보낸다
- pass: 내보낸 PNG 임의 10지점의 값이 수식 base*(1-0.6) + base*0.5*0.6 와 ±2/255 이내로 일치한다.
- Photoshop: Layers 패널 Blend Mode 드롭다운 + Opacity 슬라이더.
- canvas_entry: layer.new.layer + blend/opacity (E/new_layer.rs, E/layer_menu_cmds.rs 'blend','opacity'; U/blend_keys.rs, U/opacity_keys.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### LY-02 — 레이어 3개 그룹화 후 Merge Visible

- input: `psd/layers-styles.psd`  · mode: photo  · timed: no
- steps:
  1. 레이어 3개를 Shift 선택, Cmd/Ctrl+G 로 그룹
  2. 그룹 불투명도 50%
  3. Layer › Merge Visible
- pass: 머지 전 합성 PNG 와 머지 후 PNG 의 RMSE 가 1/255 미만이고 레이어 수가 1개가 된다.
- Photoshop: Layer › Group Layers(Cmd+G), Layer › Merge Visible(Shift+Cmd+E).
- canvas_entry: layer.new.group, layer.mergeVisible (U/menu_catalog.rs:444; E/layer_menu_cmds.rs, E/layer_multi_cmds.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### LY-03 — 레이어 스타일: Drop Shadow + Stroke 추가

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. 제품을 새 레이어로 잘라낸다(SEL-02 결과 활용)
  2. Layer › Layer Style › Drop Shadow: 거리 10, 크기 15, 불투명도 75%
  3. Stroke 3px 흰색 Outside 추가, OK
- pass: 제품 외곽 바깥 3px 띠가 흰색(≥250)이고 그 바깥 우하단 방향 15px 구간에 어두워진 픽셀(원본 대비 −20 이상)이 존재한다.
- Photoshop: Layer › Layer Style › Drop Shadow… / Stroke… (fx 버튼).
- canvas_entry: layer_style (E/layer_style.rs 'distance','spread','stroke','outside'; U/layer_style.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### LY-04 — 클리핑 마스크로 사진을 글자에 가두기

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: no
- steps:
  1. 텍스트 레이어 'CANVAS' 를 사진 아래에 만든다
  2. 사진 레이어를 선택하고 Layer › Create Clipping Mask(Cmd+Alt+G)
  3. 사진 레이어를 Move 도구로 50px 이동한다
- pass: 결과 합성에서 글자 바깥 영역 알파가 0 이고, 이동 후에도 글자 바깥은 0 으로 유지된다.
- Photoshop: Layer › Create Clipping Mask(Cmd+Alt+G) / Alt+클릭 레이어 경계.
- canvas_entry: layer.createClippingMask (U/menu_catalog.rs:338; E/clip_group.rs 'clipped','toggle')
- verdict: `works|slow|broken|missing` (Stage 2)

### LY-05 — 조정 레이어(Curves)를 비파괴로 추가하고 다시 편집

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. Layer › New Adjustment Layer › Curves…, 중간점을 +30 올리고 OK
  2. 원본 레이어 픽셀이 바뀌지 않았는지 확인한다
  3. 조정 레이어 썸네일 더블클릭으로 다시 열어 +10 으로 바꾼다
- pass: 원본 레이어 해시가 변하지 않고, 재편집 후 합성 중간톤 평균이 1차 편집보다 낮아진다(단조 반응).
- Photoshop: Layer › New Adjustment Layer › Curves…; Properties 패널에서 재편집.
- canvas_entry: layer.newAdjustmentLayer.curves (U/menu_catalog.rs:300; E/adjust_params.rs, E/adjust_cmds.rs 'curve')
- verdict: `works|slow|broken|missing` (Stage 2)

## mask (3)

### MK-01 — 선택에서 레이어 마스크 만들고 브러시로 수정

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. SEL-03 선택 상태에서 Layer › Layer Mask › Reveal Selection
  2. 마스크 썸네일을 선택하고 검정 브러시로 어깨 일부를 지운다
  3. Alt+클릭으로 마스크 단독 보기, 다시 Alt+클릭
- pass: 마스크 채널이 선택 마스크와 픽셀 일치(MAE<1/255)하고, 브러시 칭한 영역 마스크 값이 0 으로 바뀐다.
- Photoshop: Layer › Layer Mask › Reveal Selection; 마스크 썸네일 Alt+클릭 단독 보기.
- canvas_entry: layer.layerMask.revealSelection (U/menu_catalog.rs:320; E/mask_view_cmds.rs; U/mask_thumbs_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### MK-02 — 마스크 Density/Feather 비파괴 조절

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. MK-01 마스크를 선택하고 Properties 패널을 연다
  2. Density 70%, Feather 20px 로 설정한다
  3. 값을 다시 100%/0px 로 되돌린다
- pass: Density 70% 에서 마스크 바깥 영역 합성 알파가 0.30±0.02 이고, 되돌린 뒤 합성이 MK-01 결과와 해시 일치한다.
- Photoshop: Properties 패널(Masks): Density, Feather 슬라이더.
- canvas_entry: layer mask density/feather (E/layer_mask_props_cmds.rs; U/mask_props_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### MK-03 — 채널을 선택으로 불러와 알파 마스크 만들기

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: no
- steps:
  1. Channels 패널에서 Blue 채널을 Cmd/Ctrl+클릭해 선택으로 불러온다
  2. Select › Inverse
  3. Layer › Layer Mask › Reveal Selection
- pass: 생성된 마스크 값이 255 − Blue 채널 값과 전 픽셀에서 ±1 이내로 일치한다.
- Photoshop: Channels 패널 채널 썸네일 Cmd+클릭(Load Selection) → Layer Mask.
- canvas_entry: channel.target + select from channel (E/channel_cmds.rs 'layermask','channel.target'; U/channels_panel.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

## adjust (5)

### AD-01 — Levels 로 블랙/화이트 포인트 잡기

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. Image › Adjustments › Levels…(Cmd+L)
  2. Input 블랙 20, 화이트 235, 감마 1.10 입력
  3. OK
- pass: 결과 히스토그램에서 입력 20 이하 픽셀이 0 이 되고 235 이상이 255 가 되며, 수식 적용 참값과 RMSE 1/255 미만이다.
- Photoshop: Image › Adjustments › Levels…(Cmd+L): Input/Output Levels, 감마.
- canvas_entry: image.adjustments.levels (U/menu_catalog.rs:185; E/adjust_cmds.rs 'gamma','shadow','highlight')
- verdict: `works|slow|broken|missing` (Stage 2)

### AD-02 — Curves 로 S커브 대비 올리기

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. Image › Adjustments › Curves…(Cmd+M)
  2. 점 (64,48), (192,208) 두 개를 추가한다
  3. 채널을 Red 로 바꿔 (128,136) 한 점 추가, OK
- pass: RGB 곡선 적용 결과가 같은 제어점의 단조 3차 스플라인 참값과 RMSE 2/255 미만이고 Red 채널만 추가로 변한다.
- Photoshop: Image › Adjustments › Curves…(Cmd+M): 포인트 추가, 채널 드롭다운.
- canvas_entry: image.adjustments.curves (U/menu_catalog.rs:186; E/adjust_cmds.rs 'curve','points'; U/point_curve.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### AD-03 — Hue/Saturation 으로 특정 색(Reds)만 바꾸기

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. Image › Adjustments › Hue/Saturation…(Cmd+U)
  2. Edit: Reds, Hue +40, Saturation −20
  3. OK
- pass: 빨강 계열(H 345°~15°) 픽셀의 평균 Hue 가 +40°±5° 이동하고, 파랑 계열 픽셀의 Hue 변화가 2° 이내이다.
- Photoshop: Image › Adjustments › Hue/Saturation…(Cmd+U): Master/Reds… 범위 선택.
- canvas_entry: image.adjustments.hueSaturation (U/menu_catalog.rs:189; E/adjust_cmds.rs 'reds','hue','saturation')
- verdict: `works|slow|broken|missing` (Stage 2)

### AD-04 — Shadows/Highlights 로 역광 보정

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: yes
- steps:
  1. Image › Adjustments › Shadows/Highlights…
  2. Shadows Amount 50%, Highlights Amount 30%
  3. OK, 8k 전체 적용 시간을 잰다
- pass: 하위 10% 명도 픽셀 평균 L 이 15 이상 올라가고 상위 10% 는 10 이상 내려가며, 8k 적용이 5초 미만이다.
- Photoshop: Image › Adjustments › Shadows/Highlights…: Amount/Tone/Radius.
- canvas_entry: image.adjustments.shadowsHighlights (U/menu_catalog.rs:202; E/adjust_cmds.rs 'shadow','highlight','radius')
- verdict: `works|slow|broken|missing` (Stage 2)

### AD-05 — RAW(DNG) 열어 Camera Raw 에서 노출·화이트밸런스 보정

- input: `photo/raw-sample.dng`  · mode: photo  · timed: yes
- steps:
  1. File › Open 으로 DNG 를 연다(Camera Raw 다이얼로그가 떠야 함)
  2. Temperature +500K, Exposure +0.7, Dehaze 20
  3. Open 으로 문서로 가져온다
- pass: DNG 가 디모자이킹되어 열리고(회색 썸네일·오류 없음), 결과 평균 밝기가 기본 현상 대비 +0.6~0.8 EV, RAW 디코드+렌더가 10초 미만이다.
- Photoshop: File › Open(.dng) → Camera Raw: Basic 패널 Temperature/Exposure/Dehaze → Open.
- canvas_entry: filter.cameraRaw / raw open (U/camera_raw_ui.rs; E/lens_cmds.rs 'temperature','dehaze','exposure'; crates/raw)
- verdict: `works|slow|broken|missing` (Stage 2)

## filter (5)

### FL-01 — Gaussian Blur 20px 를 8k 전체에 적용

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: yes
- steps:
  1. Filter › Blur › Gaussian Blur…
  2. Radius 20px, 미리보기 켜고 OK
  3. 적용 시간을 잰다
- pass: 결과가 참값 가우시안(σ≈20/3)과 RMSE 1/255 미만이고 8k 적용이 3초 미만이다.
- Photoshop: Filter › Blur › Gaussian Blur…: Radius.
- canvas_entry: filter.blur.gaussianBlur (U/menu_catalog.rs:553; E/filters.rs 'gaussian'; U/filter_dialog.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### FL-02 — Unsharp Mask 로 선명도 올리기

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. Filter › Sharpen › Unsharp Mask…
  2. Amount 80%, Radius 1.5px, Threshold 3
  3. OK
- pass: 엣지 픽셀(Sobel>40)의 로컬 대비가 평균 30% 이상 증가하고, 평탄 영역(표준편차<3)의 변화는 1/255 이내이다.
- Photoshop: Filter › Sharpen › Unsharp Mask…: Amount/Radius/Threshold.
- canvas_entry: filter.sharpen.unsharpMask (U/menu_catalog.rs:599; E/filters.rs 'filter.sharpen.sharpen', E/filters_ext.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### FL-03 — Content-Aware Fill 로 배경 물체 지우기

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: yes
- steps:
  1. 올가미로 하늘의 새(또는 작은 물체)를 여유 있게 선택한다
  2. Edit › Content-Aware Fill…, Output: New Layer
  3. OK
- pass: 채운 영역의 색 분포(평균/표준편차)가 주변 50px 링과 5% 이내로 일치하고 원래 물체 엣지가 남지 않으며(Sobel 합 50% 감소) 새 레이어로 출력된다.
- Photoshop: Edit › Content-Aware Fill…: Sampling Area, Output To.
- canvas_entry: edit.contentAwareFill (U/menu_catalog.rs:98; E/fill_cmds.rs fn content_aware; E/retouch_cmds/remove.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### FL-04 — Lens Correction 으로 배럴 왜곡·비네팅 자동 보정

- input: `photo/raw-sample.dng`  · mode: photo  · timed: no
- steps:
  1. Filter › Lens Correction…
  2. Auto Correction: Geometric Distortion, Vignette 켬
  3. OK
- pass: 직선(수평선) 양끝과 중앙의 y 차가 보정 전 대비 70% 이상 줄고, 모서리/중앙 밝기 비가 0.9 이상이 된다.
- Photoshop: Filter › Lens Correction…: Auto Correction(EXIF 렌즈 프로필) / Custom.
- canvas_entry: filter.lensCorrection (U/menu_catalog.rs:545; E/lens_cmds.rs 'distortion','profile','exif')
- verdict: `works|slow|broken|missing` (Stage 2)

### FL-05 — 스마트 오브젝트에 Smart Filter(High Pass) 비파괴 적용

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. Layer › Smart Objects › Convert to Smart Object
  2. Filter › Other › High Pass… Radius 3px
  3. Layers 패널에서 Smart Filter 행을 더블클릭해 Radius 6 으로 수정
- pass: 스마트 오브젝트 원본 픽셀 해시가 변하지 않고, Smart Filter 재편집 후 결과 고주파 에너지가 1차보다 커진다(Laplacian 분산 증가).
- Photoshop: Layer › Smart Objects › Convert to Smart Object → Filter 적용 → Smart Filters 더블클릭 재편집.
- canvas_entry: layer.smartObjects.convertToSmartObject + filter.other.highPass (U/menu_catalog.rs:340,613; E/smart_cmds.rs 'filter','params'; U/smart_filter_rows_tests.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

## text (3)

### TX-01 — 포인트 텍스트 입력 후 폰트·크기·색 바꾸기

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. Type 도구(T)로 클릭, 'Redrob Canvas' 입력
  2. Character 패널에서 폰트 Pretendard Bold(없으면 시스템 sans), 72pt, 색 #FF3B30
  3. Esc 로 커밋
- pass: 텍스트 레이어가 편집 가능 상태로 남고, 렌더된 글리프 색이 #FF3B30 ±2, 캡 높이가 72pt 기준 ±2px 이다.
- Photoshop: Horizontal Type Tool(T) 클릭 → 입력 → Character 패널 Font/Size/Color.
- canvas_entry: type.create / type.edit (E/type_cmds.rs 'family','size','color'; U/type_tool.rs, U/type_panels_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### TX-02 — 단락 텍스트 상자에 양쪽 정렬 긴 문장 흘리기

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: no
- steps:
  1. Type 도구로 800x400 상자를 드래그한다
  2. lorem ipsum 3문장을 붙여 넣는다
  3. Paragraph 패널에서 Justify All, 행간 1.4, 하이픈 켬
- pass: 모든 행(마지막 제외)의 좌우 끝이 상자 경계와 ±1px 이내이고 상자 밖으로 넘친 글자가 없다(오버플로 표시가 아니면 통과).
- Photoshop: Type Tool 드래그(Paragraph Text) → Paragraph 패널 Justify/Hyphenate/Leading.
- canvas_entry: type.create box + type.panels.paragraph (E/type_cmds.rs 'box','justify','hyphenate','leading'; U/type_panels_ui.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### TX-03 — Warp Text(Arc) 적용 후 편집 가능 유지

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. TX-01 텍스트 레이어 선택, Type › Warp Text…
  2. Style: Arc, Bend +40%
  3. OK 후 텍스트를 'Redrob' 으로 다시 편집한다
- pass: 글자 베이스라인이 아크(중앙이 양끝보다 위, 높이차 ≥ 글자높이 0.4배)를 이루고, 편집 후에도 워프가 유지되며 레이어가 래스터화되지 않는다.
- Photoshop: Type › Warp Text…: Style(Arc…), Bend, Horizontal/Vertical Distortion.
- canvas_entry: type.warpText (U/menu_catalog.rs:485, U/warp_text_dialog_tests.rs; E/type_extra_cmds.rs 'warp','arc','bend')
- verdict: `works|slow|broken|missing` (Stage 2)

## psd (5)

### PS-01 — 레이어 스타일 PSD 열어 합성 일치 확인

- input: `psd/layers-styles.psd`  · mode: photo  · timed: no
- steps:
  1. File › Open 으로 PSD 를 연다
  2. 레이어 수, 그룹 구조, fx 표시를 Layers 패널에서 확인한다
  3. File › Export › Quick Export as PNG
- pass: 레이어/그룹 트리가 PSD 원본과 이름·순서·개수 모두 일치하고, 내보낸 PNG 가 PSD 내장 합성 미리보기와 RMSE 3/255 미만이다.
- Photoshop: File › Open(.psd); 레이어 스타일은 Layers 패널 fx 로 표시.
- canvas_entry: file.open psd (E/file_cmds.rs:122; crates/psd/src/{file,layer,filter_effects}.rs; E/layer_style.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### PS-02 — 스마트 오브젝트 PSD 열고 Edit Contents 로 수정 반영

- input: `psd/smart-object.psd`  · mode: photo  · timed: no
- steps:
  1. PSD 를 열고 스마트 오브젝트 레이어를 더블클릭(Edit Contents)
  2. 내장 문서에 빨간 사각형을 그리고 저장(Cmd+S)
  3. 부모 문서로 돌아와 썸네일과 캔버스가 갱신되는지 확인한다
- pass: 부모 문서 합성에 빨간 사각형 영역(R>200,G<60)이 나타나고 스마트 오브젝트의 변형(스케일/회전)이 유지된다.
- Photoshop: Layer › Smart Objects › Edit Contents(더블클릭) → 저장 시 부모 갱신.
- canvas_entry: layer.smartObjects.editContents (U/menu_catalog.rs:348; E/smart_cmds.rs 'updated','instances', E/smart_cmds/unpack.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### PS-03 — 조정 레이어 PSD 의 조정값 보존·재편집

- input: `psd/adjustment-layers.psd`  · mode: photo  · timed: no
- steps:
  1. PSD 를 열고 각 조정 레이어(Levels/Curves/HueSat)를 확인한다
  2. Curves 조정 레이어 썸네일을 더블클릭해 파라미터가 PSD 값 그대로인지 본다
  3. 값을 하나 바꿔 저장 후 다시 열어 유지되는지 확인한다
- pass: 열린 합성이 PSD 미리보기와 RMSE 3/255 미만이고, 조정 레이어 3종이 모두 편집 가능한 타입으로 열리며 저장→재오픈 후 바꾼 값이 유지된다.
- Photoshop: 조정 레이어는 PSD 'adjustment layer' 리소스로 저장/로드, Properties 에서 재편집.
- canvas_entry: psd adjustment layer load/save (crates/psd/src/{tagged,descriptor}.rs; E/adjust_params.rs; E/file_cmds.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### PS-04 — 텍스트 레이어 PSD 를 편집 가능 상태로 열기

- input: `psd/text-layers.psd`  · mode: photo  · timed: no
- steps:
  1. PSD 를 연다
  2. 텍스트 레이어를 Type 도구로 클릭해 글자 하나를 바꾼다
  3. 폰트가 없으면 대체 폰트 경고가 뜨는지 확인한다
- pass: 텍스트 레이어가 래스터가 아닌 편집 가능 타입으로 열리고(글자 수정 가능), 원본 폰트가 있을 때 렌더가 PSD 미리보기와 RMSE 5/255 미만이다.
- Photoshop: PSD TySh 텍스트 레이어 로드; 폰트 누락 시 Missing Fonts 대화상자.
- canvas_entry: type.psd (U/menus.rs grep 'type.psd'; E/type_cmds.rs; crates/psd/src/tagged.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### PS-05 — 8k 대형 PSD 열기·저장 왕복 시간과 손실 확인

- input: `psd/large-8k.psd`  · mode: photo  · timed: yes
- steps:
  1. PSD 를 열고 열리는 데 걸린 시간을 잰다
  2. File › Save As 로 다른 이름의 PSD 로 저장, 시간을 잰다
  3. 저장된 PSD 를 Photoshop(또는 GIMP)에서 다시 열어 비교한다
- pass: 열기 15초 미만, 저장 20초 미만이고, 재저장 파일의 레이어 수·이름·블렌드모드·마스크가 원본과 동일하며 합성 RMSE 1/255 미만이다.
- Photoshop: File › Open / File › Save As(.psd, Maximize Compatibility).
- canvas_entry: file.open / file.save psd (E/file_cmds.rs:167,177; crates/psd/src/{io,compression,builder}.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

## export (3)

### EX-01 — Export As 로 JPEG 품질 80 내보내기

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: no
- steps:
  1. File › Export › Export As…
  2. Format JPG, Quality 80, Embed Color Profile 켬
  3. Export
- pass: 출력 JPEG 가 열리고 크기가 원본 픽셀 크기와 같으며 sRGB 프로파일이 임베드되고 PSNR ≥ 38dB 이다.
- Photoshop: File › Export › Export As…: Format/Quality/Color Space.
- canvas_entry: file.export.exportAs (U/menu_catalog.rs:22, U/export_dialog.rs; E/export_preset_cmds.rs 'quality','profile')
- verdict: `works|slow|broken|missing` (Stage 2)

### EX-02 — WebP 로 50% 리사이즈 내보내기(투명 유지)

- input: `photo/product-1k.png`  · mode: photo  · timed: no
- steps:
  1. 배경을 지워 투명 영역을 만든다(SEL-02 → Delete)
  2. File › Export › Export As…, Format WebP, Scale 50%, Transparency 켬
  3. Export
- pass: 출력 WebP 가 원본의 50%±1px 크기이고 알파 채널이 있으며 투명 영역 알파가 0 이다.
- Photoshop: Export As…: WebP, Image Size Scale, Transparency.
- canvas_entry: file.export.exportAs webp (E/export_preset_cmds.rs 'webp','scale','transparency'; U/export_dialog.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### EX-03 — 8k 문서 PNG 빠른 내보내기 시간

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: yes
- steps:
  1. LY-01 처럼 레이어 2개(블렌드 Multiply) 상태로 만든다
  2. File › Export › Quick Export as PNG
  3. 시간을 잰다
- pass: 출력 PNG 가 유효하고 8k 크기이며 합성 결과와 픽셀 일치, 내보내기 완료가 8초 미만이다.
- Photoshop: File › Export › Quick Export as PNG (Export 환경설정 형식).
- canvas_entry: file.export.quickExportAsPng (U/menu_catalog.rs:21, U/menus.rs:41; E/web_cmds.rs 'png24','png32')
- verdict: `works|slow|broken|missing` (Stage 2)

## vector (2)

### VC-01 — SVG 아이콘 열어 펜 도구로 앵커 이동 후 SVG 저장

- input: `svg/icons.svg`  · mode: vector  · timed: no
- steps:
  1. 벡터 모드에서 SVG 를 연다
  2. Direct Selection 으로 앵커 하나를 선택해 20px 이동한다
  3. SVG 로 다시 저장한다
- pass: 열린 패스 개수가 원본 <path> 수와 같고, 저장된 SVG 의 해당 앵커 좌표만 20±0.5 변하며 다른 path d 는 원본과 일치한다.
- Photoshop: (Illustrator) File › Open(.svg) → Direct Selection(A) → File › Save As SVG.
- canvas_entry: vectorcraft file.open svg / path.reshape (crafts/vectorcraft/crates/engine/src/cmd/fileio/svg.rs, cmd/path.rs; crates/svg)
- verdict: `works|slow|broken|missing` (Stage 2)

### VC-02 — Pathfinder Unite 로 겹친 도형 합치기

- input: `svg/illustration-complex.svg`  · mode: vector  · timed: no
- steps:
  1. 겹친 도형 2개를 선택한다
  2. Pathfinder › Unite
  3. 결과 패스의 앵커 수를 확인한다
- pass: 결과가 단일 패스이고 면적이 두 원본 면적의 합 − 교집합과 1% 이내로 같다.
- Photoshop: (Illustrator) Window › Pathfinder › Unite.
- canvas_entry: object.pathfinder.unite (crafts/vectorcraft/crates/engine/src/cmd/pathops.rs; crates/pathops)
- verdict: `works|slow|broken|missing` (Stage 2)

## layout (1)

### LO-01 — A3 포스터 IDML 열어 PDF/X 로 내보내기

- input: `layout/poster-a3.idml`  · mode: layout  · timed: no
- steps:
  1. 레이아웃 모드에서 IDML 을 연다(없으면 PDF)
  2. 텍스트 프레임 하나의 글자를 수정한다
  3. File › Export PDF (PDF/X-4)
- pass: 페이지 크기 297x420mm±0.5mm, 폰트 전부 임베드, PDF/X 식별자 포함, 수정한 글자가 PDF 텍스트 추출에 나타난다.
- Photoshop: (InDesign) File › Open(.idml) → File › Export › Adobe PDF(Print) PDF/X-4.
- canvas_entry: designcraft file.open idml / export pdf (crafts/designcraft/crates/idml/src/import.rs, crates/pdf/src/{export,pdfx}.rs, engine/src/cmd/export.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

## ai (2)

### AI-01 — 원클릭 배경 제거(피사체 분리)

- input: `photo/portrait-4k.jpg`  · mode: photo  · timed: yes
- steps:
  1. Select › Subject(또는 Remove Background 퀵 액션)
  2. 결과 선택으로 레이어 마스크를 만든다
  3. 실행 시간을 잰다
- pass: 마스크가 인물 참값과 IoU 0.92 이상, 머리카락 영역 알파 MAE 0.2 이하, 4k 처리 5초 미만(CPU) 이다.
- Photoshop: Select › Subject / Properties › Quick Actions › Remove Background(Sensei).
- canvas_entry: select.subject, select.sky (U/menu_catalog.rs:519-520; E/smartselect_cmds.rs, E/select_extra_cmds.rs, E/cutout_cmds.rs)
- verdict: `works|slow|broken|missing` (Stage 2)

### AI-02 — 프롬프트 기반 생성형 채우기(Generative Fill)

- input: `photo/landscape-8k.jpg`  · mode: photo  · timed: yes
- steps:
  1. 하늘 일부를 올가미로 선택한다
  2. Generative Fill 입력창에 'hot air balloon' 을 넣고 생성
  3. 생성된 변형 3개 중 하나를 선택한다
- pass: 선택 영역 안에 새 물체가 생성되어 새 레이어로 들어가고(선택 밖 픽셀 변화 0), 생성 완료가 60초 미만이다.
- Photoshop: Contextual Task Bar › Generative Fill (Firefly) → 프롬프트 → Generate → Variations.
- canvas_entry: unknown (engine/ui-egui 에 generative/neural/inpaint 문자열 없음; 가장 가까운 것은 edit.contentAwareFill)
- verdict: `works|slow|broken|missing` (Stage 2)

## 입력 corpus (Stage 1-B 와 약속한 파일명)

| 파일 | 쓰는 task |
|---|---|
| `layout/poster-a3.idml` | LO-01 |
| `photo/landscape-8k.jpg` | SEL-01, SEL-04, LY-04, MK-03, AD-04, FL-01, FL-03, TX-02, EX-03, AI-02 |
| `photo/portrait-4k.jpg` | BR-04, SEL-03, SEL-06, TR-04, LY-05, MK-01, MK-02, AD-01, AD-02, FL-02, FL-05, EX-01, AI-01 |
| `photo/product-1k.png` | BR-01, BR-02, BR-03, BR-05, BR-06, SEL-02, SEL-05, TR-01, TR-02, TR-03, LY-01, LY-03, AD-03, TX-01, TX-03, EX-02 |
| `photo/raw-sample.dng` | AD-05, FL-04 |
| `psd/adjustment-layers.psd` | PS-03 |
| `psd/large-8k.psd` | PS-05 |
| `psd/layers-styles.psd` | LY-02, PS-01 |
| `psd/smart-object.psd` | PS-02 |
| `psd/text-layers.psd` | PS-04 |
| `svg/icons.svg` | VC-01 |
| `svg/illustration-complex.svg` | VC-02 |
