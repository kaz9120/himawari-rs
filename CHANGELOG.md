# Changelog

## [0.8.3](https://github.com/kaz9120/himawari-rs/compare/v0.8.2...v0.8.3) (2026-07-27)


### その他の変更

* LMRのリダクションを1024倍固定小数にする（ADR-0076） ([#22](https://github.com/kaz9120/himawari-rs/issues/22)) ([e732914](https://github.com/kaz9120/himawari-rs/commit/e7329145b2b4852968c29ed05068ca6389472827))

## [0.8.2](https://github.com/kaz9120/himawari-rs/compare/v0.8.1...v0.8.2) (2026-07-27)


### ドキュメント

* razoringのマージン見直しを機能検証で棄却する（ADR-0075） ([#20](https://github.com/kaz9120/himawari-rs/issues/20)) ([686bb4c](https://github.com/kaz9120/himawari-rs/commit/686bb4cd5ac4e7c4a49c6cb6b0d4aec9f39d521c))

## [0.8.1](https://github.com/kaz9120/himawari-rs/compare/v0.8.0...v0.8.1) (2026-07-27)


### ドキュメント

* history pruningの再挑戦を280局で打ち切る（ADR-0072） ([#18](https://github.com/kaz9120/himawari-rs/issues/18)) ([e15388f](https://github.com/kaz9120/himawari-rs/commit/e15388f01661256c8cfca68b5f9c7db9f3a308fd))

## [0.8.0](https://github.com/kaz9120/himawari-rs/compare/v0.7.9...v0.8.0) (2026-07-27)


### 棋力向上

* history bonus/malus式を再設計する（+42.2 Elo、ADR-0073） ([#16](https://github.com/kaz9120/himawari-rs/issues/16)) ([12a9ee1](https://github.com/kaz9120/himawari-rs/commit/12a9ee123496ac7d625314e4906c36c8bde47253))

## [0.7.9](https://github.com/kaz9120/himawari-rs/compare/v0.7.8...v0.7.9) (2026-07-27)


### ドキュメント

* SPRTの前に機能検証を行う規約を定める（ADR-0074） ([#14](https://github.com/kaz9120/himawari-rs/issues/14)) ([830aff5](https://github.com/kaz9120/himawari-rs/commit/830aff58d643e9114ca08aa0bc04b5d96e648e76))

## [0.7.8](https://github.com/kaz9120/himawari-rs/compare/v0.7.7...v0.7.8) (2026-07-27)


### ドキュメント

* history pruningの不発を診断し、bonus/malus式の再設計を起草する ([#12](https://github.com/kaz9120/himawari-rs/issues/12)) ([3092d3d](https://github.com/kaz9120/himawari-rs/commit/3092d3ddcaf12a909f589df882ee22212b329f20))

## [0.7.7](https://github.com/kaz9120/himawari-rs/compare/v0.7.6...v0.7.7) (2026-07-27)


### 内部

* リリースPRでCargo.lockのバージョンを同期する（ADR-0071） ([#10](https://github.com/kaz9120/himawari-rs/issues/10)) ([f095e00](https://github.com/kaz9120/himawari-rs/commit/f095e009f696430fcf78ef0e582b3ccbf0b10ef8))

## [0.7.6](https://github.com/kaz9120/himawari-rs/compare/v0.7.5...v0.7.6) (2026-07-27)


### ドキュメント

* やねうら王との探索機能差分を棚卸ししてIDEASに反映する ([#8](https://github.com/kaz9120/himawari-rs/issues/8)) ([c4e6181](https://github.com/kaz9120/himawari-rs/commit/c4e618191e49980fc58988096c5b043871f3ec68))

## [0.7.5](https://github.com/kaz9120/himawari-rs/compare/v0.7.4...v0.7.5) (2026-07-27)


### ドキュメント

* ADR-0071の記述を実装結果に合わせる ([#6](https://github.com/kaz9120/himawari-rs/issues/6)) ([f19df6d](https://github.com/kaz9120/himawari-rs/commit/f19df6d8b270006783dde3e2e4ca1bbdcca978b7))

## [0.7.4](https://github.com/kaz9120/himawari-rs/compare/v0.7.3...v0.7.4) (2026-07-27)


### 内部

* release-pleaseのrelease-typeをsimpleに変える ([#4](https://github.com/kaz9120/himawari-rs/issues/4)) ([aaeb00c](https://github.com/kaz9120/himawari-rs/commit/aaeb00c67937de2491fd038e05fe263f2bde4c62))
* バージョン更新とリリースをrelease-pleaseへ委ねる（ADR-0071） ([#3](https://github.com/kaz9120/himawari-rs/issues/3)) ([218deda](https://github.com/kaz9120/himawari-rs/commit/218dedaaed2b02e0e559695d9a1b6a16305ac3cf))
