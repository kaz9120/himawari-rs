# Changelog

## [0.10.7](https://github.com/kaz9120/himawari-rs/compare/v0.10.6...v0.10.7) (2026-07-28)


### ドキュメント

* LMRのcutNode項を棄却として記録する（ADR-0084） ([#46](https://github.com/kaz9120/himawari-rs/issues/46)) ([23af0b7](https://github.com/kaz9120/himawari-rs/commit/23af0b71d1b1e3db39591289645ab021d0f46aeb))

## [0.10.6](https://github.com/kaz9120/himawari-rs/compare/v0.10.5...v0.10.6) (2026-07-28)


### その他の変更

* MultiPVの出力スコアを直前ラインの出力値で頭打ちにする ([#43](https://github.com/kaz9120/himawari-rs/issues/43)) ([2acf78e](https://github.com/kaz9120/himawari-rs/commit/2acf78ea499ac620b721d53c4cda6d02a587f89d))


### 内部

* 機能検証をスクリプト化し局面と深さを固定する ([#42](https://github.com/kaz9120/himawari-rs/issues/42)) ([29f7d19](https://github.com/kaz9120/himawari-rs/commit/29f7d19001379f19ae2bc690ab013a027284b8b3))

## [0.10.5](https://github.com/kaz9120/himawari-rs/compare/v0.10.4...v0.10.5) (2026-07-28)


### その他の変更

* setoptionの値を元の行から切り出し、引用符を落とす ([#40](https://github.com/kaz9120/himawari-rs/issues/40)) ([3cb18b1](https://github.com/kaz9120/himawari-rs/commit/3cb18b1d58e6f7b888736fbec2b401b7340b0058))

## [0.10.4](https://github.com/kaz9120/himawari-rs/compare/v0.10.3...v0.10.4) (2026-07-28)


### ドキュメント

* book-v1の公開を記録しADR-0082の判断を実際に合わせる ([#38](https://github.com/kaz9120/himawari-rs/issues/38)) ([8b40611](https://github.com/kaz9120/himawari-rs/commit/8b406119c8ac56088a4790b57143d67b06d4f4f4))

## [0.10.3](https://github.com/kaz9120/himawari-rs/compare/v0.10.2...v0.10.3) (2026-07-28)


### その他の変更

* WindowsバイナリをMSVCランタイム静的リンクで配布する（ADR-0083） ([#36](https://github.com/kaz9120/himawari-rs/issues/36)) ([f6684ff](https://github.com/kaz9120/himawari-rs/commit/f6684ff98e7be811cd4fc0ceb8d9670f036f034d))

## [0.10.2](https://github.com/kaz9120/himawari-rs/compare/v0.10.1...v0.10.2) (2026-07-28)


### 内部

* 定跡の配布と生成条件の記録を整える（ADR-0082） ([#34](https://github.com/kaz9120/himawari-rs/issues/34)) ([4db7441](https://github.com/kaz9120/himawari-rs/commit/4db74414a0df9107e25b19c8a3620764f67c705b))

## [0.10.1](https://github.com/kaz9120/himawari-rs/compare/v0.10.0...v0.10.1) (2026-07-28)


### 内部

* 開発環境の再現手順をスクリプト化する（ADR-0080・0081） ([#30](https://github.com/kaz9120/himawari-rs/issues/30)) ([aa8716a](https://github.com/kaz9120/himawari-rs/commit/aa8716a1f95396ea6ff155e05d5755b78b16dfe9))

## [0.10.0](https://github.com/kaz9120/himawari-rs/compare/v0.9.0...v0.10.0) (2026-07-28)


### 棋力向上

* 置換表の下界による簡易ProbCutを入れる（+15.6 Elo、ADR-0078） ([#31](https://github.com/kaz9120/himawari-rs/issues/31)) ([42e97f9](https://github.com/kaz9120/himawari-rs/commit/42e97f977e640ac3e034ec5b2f8669db3184b134))

## [0.9.0](https://github.com/kaz9120/himawari-rs/compare/v0.8.5...v0.9.0) (2026-07-27)


### 棋力向上

* 静止探索にfutility枝刈りを入れる（+57.3 Elo、ADR-0077） ([#28](https://github.com/kaz9120/himawari-rs/issues/28)) ([02bc6e7](https://github.com/kaz9120/himawari-rs/commit/02bc6e76cebc290b0b585af9c26da4dd4be9297e))

## [0.8.5](https://github.com/kaz9120/himawari-rs/compare/v0.8.4...v0.8.5) (2026-07-27)


### その他の変更

* MultiPVの降順保証を並べ替えから出力スコアの頭打ちへ変える ([#26](https://github.com/kaz9120/himawari-rs/issues/26)) ([eee081f](https://github.com/kaz9120/himawari-rs/commit/eee081f9c2054d6926282845751cb66584268057))

## [0.8.4](https://github.com/kaz9120/himawari-rs/compare/v0.8.3...v0.8.4) (2026-07-27)


### その他の変更

* MultiPVの出力をスコア降順に整える（ADR-0032） ([#24](https://github.com/kaz9120/himawari-rs/issues/24)) ([15380d6](https://github.com/kaz9120/himawari-rs/commit/15380d6b583e73e9ef8c7dad83fb63def7551232))

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
