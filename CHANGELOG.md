# Changelog

## [0.15.4](https://github.com/kaz9120/himawari-rs/compare/v0.15.3...v0.15.4) (2026-07-29)


### その他の変更

* 深さ1のイテレーションを終えるまでstopを無視する ([#101](https://github.com/kaz9120/himawari-rs/issues/101)) ([4446c8d](https://github.com/kaz9120/himawari-rs/commit/4446c8d02b91cd83cc8717623b560358344a0382))


### ドキュメント

* 時間配分のmove horizon化を棄却した記録を残す（ADR-0102） ([#98](https://github.com/kaz9120/himawari-rs/issues/98)) ([aef504d](https://github.com/kaz9120/himawari-rs/commit/aef504db8a23a3519ebbf6a882f33b2e059207aa))


### 内部

* selfplayに両者ponderのモードを足す（--ponder-both） ([#99](https://github.com/kaz9120/himawari-rs/issues/99)) ([fd4bdb3](https://github.com/kaz9120/himawari-rs/commit/fd4bdb352ffc6e53d07a0ff30760120ba08baca9))

## [0.15.3](https://github.com/kaz9120/himawari-rs/compare/v0.15.2...v0.15.3) (2026-07-29)


### 内部

* NPS計測とプロファイルの手順をスクリプトへ固める（ADR-0081） ([#96](https://github.com/kaz9120/himawari-rs/issues/96)) ([ca0db43](https://github.com/kaz9120/himawari-rs/commit/ca0db43fcb243c02b2e4b791c1ff9908279a0608))

## [0.15.2](https://github.com/kaz9120/himawari-rs/compare/v0.15.1...v0.15.2) (2026-07-29)


### その他の変更

* MoveListのゼロ埋めをやめる（ADR-0101） ([#92](https://github.com/kaz9120/himawari-rs/issues/92)) ([4723258](https://github.com/kaz9120/himawari-rs/commit/47232587944ff9250b6b55e7b3c3e99ad6233388))

## [0.15.1](https://github.com/kaz9120/himawari-rs/compare/v0.15.0...v0.15.1) (2026-07-29)


### その他の変更

* 新規開始のsprt-run.shが空配列の展開で落ちるのを直す ([#93](https://github.com/kaz9120/himawari-rs/issues/93)) ([ba6ade7](https://github.com/kaz9120/himawari-rs/commit/ba6ade76913072155978fc91c0d92850f71e683a))

## [0.15.0](https://github.com/kaz9120/himawari-rs/compare/v0.14.0...v0.15.0) (2026-07-29)


### 棋力向上

* 指し手の最大スコア探索をSoA＋SIMDにする（+106.7 Elo、ADR-0100） ([#89](https://github.com/kaz9120/himawari-rs/issues/89)) ([794895a](https://github.com/kaz9120/himawari-rs/commit/794895a53f83edb8853b7d105d2a6485b0735f68))

## [0.14.0](https://github.com/kaz9120/himawari-rs/compare/v0.13.9...v0.14.0) (2026-07-29)


### 棋力向上

* NNUE隠れ層の内積を専用命令で計算する（+59.7 Elo、ADR-0099） ([#88](https://github.com/kaz9120/himawari-rs/issues/88)) ([a67c2c5](https://github.com/kaz9120/himawari-rs/commit/a67c2c5728a9783070cfdb44fedca05824f77b3b))

## [0.13.9](https://github.com/kaz9120/himawari-rs/compare/v0.13.8...v0.13.9) (2026-07-29)


### ドキュメント

* 次の入口をNPSのプロファイルにする（ROADMAP） ([#86](https://github.com/kaz9120/himawari-rs/issues/86)) ([4efdf28](https://github.com/kaz9120/himawari-rs/commit/4efdf28866569c81505e9dded559f1eb8d42c733))

## [0.13.8](https://github.com/kaz9120/himawari-rs/compare/v0.13.7...v0.13.8) (2026-07-29)


### ドキュメント

* capture historyの再挑戦を棄却として記録する（ADR-0097） ([#84](https://github.com/kaz9120/himawari-rs/issues/84)) ([2b18c0c](https://github.com/kaz9120/himawari-rs/commit/2b18c0c2a32c0e2632805ae1a1f1a5d0c76309e9))

## [0.13.7](https://github.com/kaz9120/himawari-rs/compare/v0.13.6...v0.13.7) (2026-07-29)


### 内部

* SPRTの再開を自動化する（ADR-0087） ([#82](https://github.com/kaz9120/himawari-rs/issues/82)) ([9f09b1b](https://github.com/kaz9120/himawari-rs/commit/9f09b1b53464aa048334548bee333236b8d4dea0))

## [0.13.6](https://github.com/kaz9120/himawari-rs/compare/v0.13.5...v0.13.6) (2026-07-29)


### ドキュメント

* 待機と判断を分ける運用上の注意を足す（ADR-0098） ([#80](https://github.com/kaz9120/himawari-rs/issues/80)) ([2ac400d](https://github.com/kaz9120/himawari-rs/commit/2ac400d857bac901daed7664e8151b585825c94c))

## [0.13.5](https://github.com/kaz9120/himawari-rs/compare/v0.13.4...v0.13.5) (2026-07-29)


### 内部

* SPRTの手順をスクリプトへ固める（ADR-0081） ([#78](https://github.com/kaz9120/himawari-rs/issues/78)) ([bfe2ea2](https://github.com/kaz9120/himawari-rs/commit/bfe2ea2032f20f30876b41a5e4e93f8cf2cb4c6a))

## [0.13.4](https://github.com/kaz9120/himawari-rs/compare/v0.13.3...v0.13.4) (2026-07-29)


### 内部

* エージェントが待機で止まらないようにする（ADR-0098） ([#76](https://github.com/kaz9120/himawari-rs/issues/76)) ([750e349](https://github.com/kaz9120/himawari-rs/commit/750e3495f82edaa0f48fd10aafc9eb06f9319b14))

## [0.13.3](https://github.com/kaz9120/himawari-rs/compare/v0.13.2...v0.13.3) (2026-07-29)


### ドキュメント

* RFPのマージン緩和を棄却として記録する（ADR-0096） ([#74](https://github.com/kaz9120/himawari-rs/issues/74)) ([0a01b0a](https://github.com/kaz9120/himawari-rs/commit/0a01b0ab09ad53630baedc3c6a25aaf57c131230))

## [0.13.2](https://github.com/kaz9120/himawari-rs/compare/v0.13.1...v0.13.2) (2026-07-29)


### その他の変更

* SEEで初手の成りを扱う（ADR-0095） ([#72](https://github.com/kaz9120/himawari-rs/issues/72)) ([9535517](https://github.com/kaz9120/himawari-rs/commit/953551775143a75ebb509b65998a79f729e1a420))

## [0.13.1](https://github.com/kaz9120/himawari-rs/compare/v0.13.0...v0.13.1) (2026-07-28)


### その他の変更

* mate_1plyの検証を軽くする（ADR-0094） ([#70](https://github.com/kaz9120/himawari-rs/issues/70)) ([6fd85c4](https://github.com/kaz9120/himawari-rs/commit/6fd85c435d78d562a0cc7a8c5b4e62eee689ab75))

## [0.13.0](https://github.com/kaz9120/himawari-rs/compare/v0.12.3...v0.13.0) (2026-07-28)


### 棋力向上

* SEEを駒打ちに対応させる（+67.0 Elo、ADR-0091） ([#59](https://github.com/kaz9120/himawari-rs/issues/59)) ([e578957](https://github.com/kaz9120/himawari-rs/commit/e578957b587db00e8bdb62d4ec947d75c4ad20e1))


### ドキュメント

* ブランチ運用の規約を足す（ADR-0070） ([#68](https://github.com/kaz9120/himawari-rs/issues/68)) ([038e957](https://github.com/kaz9120/himawari-rs/commit/038e957f01998397cfa4f60cac63ea5857cc4b18))

## [0.12.3](https://github.com/kaz9120/himawari-rs/compare/v0.12.2...v0.12.3) (2026-07-28)


### その他の変更

* 詰まされる側でも反復深化を打ち切る（ADR-0088） ([#66](https://github.com/kaz9120/himawari-rs/issues/66)) ([649586c](https://github.com/kaz9120/himawari-rs/commit/649586c2265e1da254d52f95892c912de9bf1ccc))

## [0.12.2](https://github.com/kaz9120/himawari-rs/compare/v0.12.1...v0.12.2) (2026-07-28)


### ドキュメント

* 詰まされる側を打ち切らない根拠を実測で補う（ADR-0088） ([#64](https://github.com/kaz9120/himawari-rs/issues/64)) ([9b30cc9](https://github.com/kaz9120/himawari-rs/commit/9b30cc9d943a2c676ff4f7d9c63ea053dd653a08))

## [0.12.1](https://github.com/kaz9120/himawari-rs/compare/v0.12.0...v0.12.1) (2026-07-28)


### ドキュメント

* ROADMAPの現在地を2026-07-29時点へ更新する ([#62](https://github.com/kaz9120/himawari-rs/issues/62)) ([00b0c35](https://github.com/kaz9120/himawari-rs/commit/00b0c35f29b752145eb763c3c195dc39f377d2c7))

## [0.12.0](https://github.com/kaz9120/himawari-rs/compare/v0.11.1...v0.12.0) (2026-07-28)


### 棋力向上

* lmrDepth基準を導入しSEEベースの枝刈りを入れる（+45.6 Elo、ADR-0090） ([#54](https://github.com/kaz9120/himawari-rs/issues/54)) ([ed1eeeb](https://github.com/kaz9120/himawari-rs/commit/ed1eeeb7aefd7dfb3b8112f55dac78070d02529e))

## [0.11.1](https://github.com/kaz9120/himawari-rs/compare/v0.11.0...v0.11.1) (2026-07-28)


### その他の変更

* aspirationのfail high/lowをinfoで報告する（ADR-0092） ([#58](https://github.com/kaz9120/himawari-rs/issues/58)) ([6d51068](https://github.com/kaz9120/himawari-rs/commit/6d5106837e8535d4594cc238793092151fce17a7))

## [0.11.0](https://github.com/kaz9120/himawari-rs/compare/v0.10.10...v0.11.0) (2026-07-28)


### 棋力向上

* correction historyを3系統に増やす（+17.7 Elo、ADR-0085） ([#56](https://github.com/kaz9120/himawari-rs/issues/56)) ([2d2709e](https://github.com/kaz9120/himawari-rs/commit/2d2709ea2b2595cea6bbed8d0e1fa7b38c685b6c))

## [0.10.10](https://github.com/kaz9120/himawari-rs/compare/v0.10.9...v0.10.10) (2026-07-28)


### ドキュメント

* 探索改善の選定基準を3軸で置く（ADR-0089） ([#53](https://github.com/kaz9120/himawari-rs/issues/53)) ([214cd77](https://github.com/kaz9120/himawari-rs/commit/214cd77a8515536c6d539cca460a50d995ac864c))

## [0.10.9](https://github.com/kaz9120/himawari-rs/compare/v0.10.8...v0.10.9) (2026-07-28)


### その他の変更

* 勝ちの詰みを見つけたら反復深化を打ち切る（ADR-0088） ([#51](https://github.com/kaz9120/himawari-rs/issues/51)) ([f6f97d6](https://github.com/kaz9120/himawari-rs/commit/f6f97d61e083376e5319432bd874b6b0cce54bac))


### 内部

* 中断したSPRTを棋譜から再開できるようにする（ADR-0087） ([#49](https://github.com/kaz9120/himawari-rs/issues/49)) ([f86604c](https://github.com/kaz9120/himawari-rs/commit/f86604cdd5caff99116b6f52b12eb94a5fb3412f))

## [0.10.8](https://github.com/kaz9120/himawari-rs/compare/v0.10.7...v0.10.8) (2026-07-28)


### その他の変更

* USIのinfoにseldepthとcurrmoveを出す（ADR-0086） ([#48](https://github.com/kaz9120/himawari-rs/issues/48)) ([fde0fa8](https://github.com/kaz9120/himawari-rs/commit/fde0fa893bde91f9c72163f547638b6197befab1))

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
