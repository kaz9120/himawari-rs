# 0003: ツールチェイン方針

- Status: accepted
- Date: 2026-07-17
- 関連ADR: なし

## Context

SIMD（NNUE推論）、PEXT（利き生成の候補）、const評価（テーブル生成）など、
言語機能の選択がツールチェインの制約に直結する。stable縛りにするか、
nightlyを許容するかを最初に決める。

開発機はApple Silicon（NEON）、対局・学習はx86_64（AVX2）も想定する。
このISA差をSIMDコードでどう吸収するかが最大の論点になる。

## 選択肢と比較

### 案A: stable固定

SIMDは `std::arch` のstable intrinsics（AVX2/SSE4.1/NEON）＋ `cfg` 分岐で書く。
ツールチェイン更新で壊れる心配がない。そのかわりISAごとに実装を書き、
薄い抽象層を自前で用意することになる。

### 案B: nightly許容

`std::simd`（portable SIMD）でISA差を吸収でき、SIMDコードを1本化できる。
const評価などの未安定機能も使える。個人開発でツールチェイン起因の破損に
対応するコストは、日付固定のピン留めでほぼ抑えられる。
生成コードがintrinsics直書きに劣る箇所が出た場合は、その箇所だけ
`std::arch` に差し替えればよい。

## Decision

案Bを採用する。具体的には次のとおり。

- `rust-toolchain.toml` で日付固定のnightlyにピン留めする（現在 nightly-2026-07-17）。
  更新は意図的に行い、CIと同一バージョンを保つ
- MSRVは定めない。ツールチェインは `rust-toolchain.toml` を正とする
- エディションは2024
- SIMDは `std::simd` を第一候補とする。性能が出ない箇所に限り
  `std::arch` intrinsicsへ差し替える（詳細はSIMD抽象化のADR、P4で起草）
- リリースプロファイルは `lto = true`, `codegen-units = 1`
- ローカルの計測・対局では `RUSTFLAGS="-C target-cpu=native"` を使う。
  CPU別の配布バイナリ（x64-avx2 / x64-sse42 / macos-arm64）はリリース時に
  GitHub Actionsで `target-feature` を固定してビルドする

## Consequences

- SIMDコードを1本化でき、ISA別実装の保守コストが下がる。
  スカラー基準実装は一致テストの基準として必ず併置する
- nightlyの更新で警告・エラーの増えることがある。ピン留めの日付更新は
  独立したコミットで行い、壊れたらすぐ戻せるようにする
- PEXT（BMI2）は `_pext_u64` で使えるため、利き生成方式のADR（P1で起草）の
  選択肢を狭めない
- `std::simd` の生成コードが目標性能に届かない場合、該当箇所を
  `std::arch` に差し替える。全面的に届かない場合はこのADRを見直す
