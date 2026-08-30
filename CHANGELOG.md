# Changelog

## [0.44.11](https://github.com/kaz9120/himawari-rs/compare/v0.44.10...v0.44.11) (2026-08-30)


### その他の変更

* EvalHashのエントリ数をビルド時のノブで絞れる形にする ([#433](https://github.com/kaz9120/himawari-rs/issues/433)) ([ae4628f](https://github.com/kaz9120/himawari-rs/commit/ae4628fff1a60dcd19e2828634c00a49d4992edc))

## [0.44.10](https://github.com/kaz9120/himawari-rs/compare/v0.44.9...v0.44.10) (2026-08-30)


### その他の変更

* ネット読み込みのピークメモリを定常分まで下げる（issue [#428](https://github.com/kaz9120/himawari-rs/issues/428)） ([#431](https://github.com/kaz9120/himawari-rs/issues/431)) ([3042549](https://github.com/kaz9120/himawari-rs/commit/304254960a0bda5ddbfe7ef909afb22c446941bb))

## [0.44.9](https://github.com/kaz9120/himawari-rs/compare/v0.44.8...v0.44.9) (2026-08-30)


### その他の変更

* rankのWorker構築をquiet_workerへ共通化する ([#425](https://github.com/kaz9120/himawari-rs/issues/425)) ([bb0ee9e](https://github.com/kaz9120/himawari-rs/commit/bb0ee9e9903a25ed452727e469e3bbd7890ee5e0))

## [0.44.8](https://github.com/kaz9120/himawari-rs/compare/v0.44.7...v0.44.8) (2026-08-30)


### その他の変更

* sprt showが対局開始前のログをエラーにしない ([#419](https://github.com/kaz9120/himawari-rs/issues/419)) ([8276cc1](https://github.com/kaz9120/himawari-rs/commit/8276cc13d010262209f430d85116612b45be8f7d))

## [0.44.7](https://github.com/kaz9120/himawari-rs/compare/v0.44.6...v0.44.7) (2026-08-30)


### その他の変更

* 学習ループを等価な書き換えで22%速くする ([#417](https://github.com/kaz9120/himawari-rs/issues/417)) ([9543464](https://github.com/kaz9120/himawari-rs/commit/9543464cfad085aeb9a6ecdb66b9793dfcb16468))

## [0.44.6](https://github.com/kaz9120/himawari-rs/compare/v0.44.5...v0.44.6) (2026-08-30)


### その他の変更

* psv quietの並列化と既定max-pliesの統一（前処理事故の再発防止） ([#413](https://github.com/kaz9120/himawari-rs/issues/413)) ([3d76ed6](https://github.com/kaz9120/himawari-rs/commit/3d76ed64ed5b970cf8b2bf684c89337cb22963cb))

## [0.44.5](https://github.com/kaz9120/himawari-rs/compare/v0.44.4...v0.44.5) (2026-08-29)


### その他の変更

* sprt netに--cand-binを足し、入力特徴の違うビルド同士を測れるようにする ([#404](https://github.com/kaz9120/himawari-rs/issues/404)) ([bbb416f](https://github.com/kaz9120/himawari-rs/commit/bbb416f4124f7cc1bead1b939a0968870ff58e2f))

## [0.44.4](https://github.com/kaz9120/himawari-rs/compare/v0.44.3...v0.44.4) (2026-08-29)


### その他の変更

* 盲点ベンチマークの測定コマンドと初回構築の記録（ADR-0191） ([#402](https://github.com/kaz9120/himawari-rs/issues/402)) ([e86a737](https://github.com/kaz9120/himawari-rs/commit/e86a737798adaf9c567314b24a8231f17168fd83))

## [0.44.3](https://github.com/kaz9120/himawari-rs/compare/v0.44.2...v0.44.3) (2026-08-29)


### その他の変更

* HalfKA入力拡張をfeatureで実装する（ADR-0193） ([#400](https://github.com/kaz9120/himawari-rs/issues/400)) ([d266e71](https://github.com/kaz9120/himawari-rs/commit/d266e7102810301d9413f0407cc1b3c4bee24161))

## [0.44.2](https://github.com/kaz9120/himawari-rs/compare/v0.44.1...v0.44.2) (2026-08-29)


### その他の変更

* 教師データの診断・取得・整形の道具を足し、ADR-0190〜0192を記録する ([#398](https://github.com/kaz9120/himawari-rs/issues/398)) ([d74f9c6](https://github.com/kaz9120/himawari-rs/commit/d74f9c6bc0693dac6061a67a039640e1f4cec051))

## [0.44.1](https://github.com/kaz9120/himawari-rs/compare/v0.44.0...v0.44.1) (2026-08-29)


### その他の変更

* sprtの--setで未知の鍵を拒否し、見送り上限の--max-pairsを足す ([#396](https://github.com/kaz9120/himawari-rs/issues/396)) ([bad12a3](https://github.com/kaz9120/himawari-rs/commit/bad12a39d78352c11c64f05607a60cde1724c9c9))

## [0.44.0](https://github.com/kaz9120/himawari-rs/compare/v0.43.3...v0.44.0) (2026-08-28)


### 棋力向上

* 自己生成の世代ループ第1周のネットへ切り替える（+13.9 Elo、ADR-0188） ([#394](https://github.com/kaz9120/himawari-rs/issues/394)) ([533f4ae](https://github.com/kaz9120/himawari-rs/commit/533f4aef0823fd9cf918d9c5ca4a53b44a88f80f))

## [0.43.3](https://github.com/kaz9120/himawari-rs/compare/v0.43.2...v0.43.3) (2026-08-27)


### その他の変更

* 成果物の掃除をhmwr cleanと保持方針ADR-0189に固定する ([#392](https://github.com/kaz9120/himawari-rs/issues/392)) ([bac02a8](https://github.com/kaz9120/himawari-rs/commit/bac02a854384062707faecb2d6dc70660d7ebef9))

## [0.43.2](https://github.com/kaz9120/himawari-rs/compare/v0.43.1...v0.43.2) (2026-08-27)


### その他の変更

* gensfenのmoveフィールドをやねうら王符号で書く ([#388](https://github.com/kaz9120/himawari-rs/issues/388)) ([b904dea](https://github.com/kaz9120/himawari-rs/commit/b904deacdc6cff7452fed2c1d38cd7e0c453674b))

## [0.43.1](https://github.com/kaz9120/himawari-rs/compare/v0.43.0...v0.43.1) (2026-08-26)


### その他の変更

* 世代ループの再設計に向けてgensfenへ--openingsを足し、ADR-0188を起草する ([#386](https://github.com/kaz9120/himawari-rs/issues/386)) ([0f0e379](https://github.com/kaz9120/himawari-rs/commit/0f0e3796fcaf7712e3f6044080cf93547cef4817))

## [0.43.0](https://github.com/kaz9120/himawari-rs/compare/v0.42.1...v0.43.0) (2026-08-26)


### 棋力向上

* 兄弟局面のランキング損失で学習したネットへ切り替える（+28.9 Elo、ADR-0185） ([#382](https://github.com/kaz9120/himawari-rs/issues/382)) ([edad15f](https://github.com/kaz9120/himawari-rs/commit/edad15f7163d38b0f52bd7b2b6d83c7bbe8b4051))

## [0.42.1](https://github.com/kaz9120/himawari-rs/compare/v0.42.0...v0.42.1) (2026-08-25)


### その他の変更

* hmwr net reorderが引数リストでft_reorderを呼べるようにする ([#380](https://github.com/kaz9120/himawari-rs/issues/380)) ([062b189](https://github.com/kaz9120/himawari-rs/commit/062b189a223151dfab10da4fe4508c97b0c6a8da))

## [0.42.0](https://github.com/kaz9120/himawari-rs/compare/v0.41.0...v0.42.0) (2026-08-22)


### 棋力向上

* SPSA第1群の探索定数24項目を焼き込む（+32.0 Elo、ADR-0143） ([#366](https://github.com/kaz9120/himawari-rs/issues/366)) ([6668a54](https://github.com/kaz9120/himawari-rs/commit/6668a543b9d6504369ecffdda2bd5770b039f9e7))


### その他の変更

* EvalFile未設定のisreadyを起動エラーで止める（ADR-0037） ([#364](https://github.com/kaz9120/himawari-rs/issues/364)) ([4e31d8c](https://github.com/kaz9120/himawari-rs/commit/4e31d8cedfb30c9353c35ba21c07cdb17d556a30))

## [0.41.0](https://github.com/kaz9120/himawari-rs/compare/v0.40.0...v0.41.0) (2026-08-19)


### 棋力向上

* 香の不成を2段目から3段目以降へ移す（ADR-0176） ([#348](https://github.com/kaz9120/himawari-rs/issues/348)) ([fa32058](https://github.com/kaz9120/himawari-rs/commit/fa3205887b6d68cbd878c0ad43e598eaea7eb87d))

## [0.40.0](https://github.com/kaz9120/himawari-rs/compare/v0.39.0...v0.40.0) (2026-08-17)


### 棋力向上

* 桂の3段目不成を通常の指し手生成でも出す（+11.2 Elo、ADR-0173） ([#342](https://github.com/kaz9120/himawari-rs/issues/342)) ([2f51b09](https://github.com/kaz9120/himawari-rs/commit/2f51b09cde1914bef945e482f68d303d21761310))

## [0.39.0](https://github.com/kaz9120/himawari-rs/compare/v0.38.0...v0.39.0) (2026-08-17)


### 棋力向上

* FT出力の対を掛けて駒対の相互作用を入れる（+65.4 Elo、ADR-0171） ([#337](https://github.com/kaz9120/himawari-rs/issues/337)) ([e02c5da](https://github.com/kaz9120/himawari-rs/commit/e02c5dadb6ccf400165b38695240e04cdc41742a))

## [0.38.0](https://github.com/kaz9120/himawari-rs/compare/v0.37.0...v0.38.0) (2026-08-16)


### 棋力向上

* 後段のL1を16へ半減する（+13.4 Elo、ADR-0170） ([#335](https://github.com/kaz9120/himawari-rs/issues/335)) ([3135499](https://github.com/kaz9120/himawari-rs/commit/3135499beec2204e17e5c53c3026259e855c3916))

## [0.37.0](https://github.com/kaz9120/himawari-rs/compare/v0.36.3...v0.37.0) (2026-08-15)


### 棋力向上

* 既定の構成をFT1024へ切り替える（ADR-0159） ([#332](https://github.com/kaz9120/himawari-rs/issues/332)) ([92ddb95](https://github.com/kaz9120/himawari-rs/commit/92ddb9550ada11b534848709b094415b52ebba04))

## [0.36.3](https://github.com/kaz9120/himawari-rs/compare/v0.36.2...v0.36.3) (2026-08-15)


### その他の変更

* MovePickerの生成段を切り出してフレームを縮める（+0.83% NPS、ADR-0166） ([#325](https://github.com/kaz9120/himawari-rs/issues/325)) ([5dd7d98](https://github.com/kaz9120/himawari-rs/commit/5dd7d98a573d905dc78d5d84b3f7b470747b7215))

## [0.36.2](https://github.com/kaz9120/himawari-rs/compare/v0.36.1...v0.36.2) (2026-08-15)


### その他の変更

* BonaPiece集合をブロック単位のレイアウトへ変える（+1.07% NPS、ADR-0165） ([#323](https://github.com/kaz9120/himawari-rs/issues/323)) ([ce8780c](https://github.com/kaz9120/himawari-rs/commit/ce8780c8ae093b481e8643ca9f4a1d4df260f920))

## [0.36.1](https://github.com/kaz9120/himawari-rs/compare/v0.36.0...v0.36.1) (2026-08-15)


### その他の変更

* BonaPiece集合の構築をブロック配置へ置き換える（ADR-0164） ([#320](https://github.com/kaz9120/himawari-rs/issues/320)) ([95a88fe](https://github.com/kaz9120/himawari-rs/commit/95a88febe6ac94629af44eabd928e15c7263df48))
* 盤上駒キーの走査重複とmakenetの位置づけを直す ([#321](https://github.com/kaz9120/himawari-rs/issues/321)) ([a7dc58f](https://github.com/kaz9120/himawari-rs/commit/a7dc58f1aa890cb23830a2bd8dd77ba6a34fc37d))

## [0.36.0](https://github.com/kaz9120/himawari-rs/compare/v0.35.20...v0.36.0) (2026-08-14)


### 棋力向上

* pawn/correction historyを全スレッドで共有する（ADR-0162） ([f72c3ac](https://github.com/kaz9120/himawari-rs/commit/f72c3ac6cef8447cd80bc935f0ffd03a168b25a1))

## [0.35.20](https://github.com/kaz9120/himawari-rs/compare/v0.35.19...v0.35.20) (2026-08-14)


### ドキュメント

* CLAUDE.mdへ測定と文書の規律を書き足す ([#312](https://github.com/kaz9120/himawari-rs/issues/312)) ([96884f8](https://github.com/kaz9120/himawari-rs/commit/96884f855f4acc0cc2ff63e4fe5959e8f1902c45))

## [0.35.19](https://github.com/kaz9120/himawari-rs/compare/v0.35.18...v0.35.19) (2026-08-14)


### ドキュメント

* ドキュメントを実態に合わせて全面的に見直す ([#310](https://github.com/kaz9120/himawari-rs/issues/310)) ([f76daa0](https://github.com/kaz9120/himawari-rs/commit/f76daa0220a0d727ff76f8245e55aa3decdde5d1))

## [0.35.18](https://github.com/kaz9120/himawari-rs/compare/v0.35.17...v0.35.18) (2026-08-14)


### ドキュメント

* FT1024を保留し、ADR-0160を群単位の参照追従へ書き直す ([#308](https://github.com/kaz9120/himawari-rs/issues/308)) ([cdb9f5f](https://github.com/kaz9120/himawari-rs/commit/cdb9f5fb56969282a02e1b44d1ed985947679d5b))

## [0.35.17](https://github.com/kaz9120/himawari-rs/compare/v0.35.16...v0.35.17) (2026-08-13)


### ドキュメント

* FT幅1024の測定を記録し、次の一手をADR-0160に起票する ([#306](https://github.com/kaz9120/himawari-rs/issues/306)) ([8389d6b](https://github.com/kaz9120/himawari-rs/commit/8389d6b417309527b8a4cec6a291f5e76598012c))

## [0.35.16](https://github.com/kaz9120/himawari-rs/compare/v0.35.15...v0.35.16) (2026-08-12)


### ドキュメント

* 左右ミラーの2案を棄却として記録する（ADR-0157・0158） ([#304](https://github.com/kaz9120/himawari-rs/issues/304)) ([86a6839](https://github.com/kaz9120/himawari-rs/commit/86a6839729f45236fca038fbf0871e20e27bf1c4))

## [0.35.15](https://github.com/kaz9120/himawari-rs/compare/v0.35.14...v0.35.15) (2026-08-11)


### ドキュメント

* 玉位置の左右ミラーを棄却として記録する（ADR-0157） ([#302](https://github.com/kaz9120/himawari-rs/issues/302)) ([fd9d1f8](https://github.com/kaz9120/himawari-rs/commit/fd9d1f88986d8050c6e6811a6bb6ce8b8a00d67c))

## [0.35.14](https://github.com/kaz9120/himawari-rs/compare/v0.35.13...v0.35.14) (2026-08-11)


### ドキュメント

* ADR-0156にFT幅ごとの取り分の実測を追記する ([#299](https://github.com/kaz9120/himawari-rs/issues/299)) ([b39b1cf](https://github.com/kaz9120/himawari-rs/commit/b39b1cfa0fb3fdb5606bc41816e08f349c50963f))

## [0.35.13](https://github.com/kaz9120/himawari-rs/compare/v0.35.12...v0.35.13) (2026-08-11)


### その他の変更

* FTの全計算を玉位置ごとのキャッシュ差分にする（ADR-0156、+2.66% NPS） ([#297](https://github.com/kaz9120/himawari-rs/issues/297)) ([8f3a5ec](https://github.com/kaz9120/himawari-rs/commit/8f3a5ec3d28d6975ac7327ddd09db6cfaeb9052b))

## [0.35.12](https://github.com/kaz9120/himawari-rs/compare/v0.35.11...v0.35.12) (2026-08-11)


### ドキュメント

* ADR-0155群1最小形の棄却を記録し群1〜3を閉じる ([#295](https://github.com/kaz9120/himawari-rs/issues/295)) ([a5b04e5](https://github.com/kaz9120/himawari-rs/commit/a5b04e55bc6e72e3628702cbec8b2f8aae6c0d40))

## [0.35.11](https://github.com/kaz9120/himawari-rs/compare/v0.35.10...v0.35.11) (2026-08-11)


### ドキュメント

* ADR-0155群1〜3の測定結果と見送り判断を記録する ([#293](https://github.com/kaz9120/himawari-rs/issues/293)) ([b7bcf48](https://github.com/kaz9120/himawari-rs/commit/b7bcf48fe007ff7dfa5a6f70aa278e08d64a6334))

## [0.35.10](https://github.com/kaz9120/himawari-rs/compare/v0.35.9...v0.35.10) (2026-08-10)


### ドキュメント

* 参照実装ウォークスルーの結果をADR-0155に起草する ([#291](https://github.com/kaz9120/himawari-rs/issues/291)) ([e6ead93](https://github.com/kaz9120/himawari-rs/commit/e6ead93a781fdb38822c3b7ba881e74e3a1e611f))

## [0.35.9](https://github.com/kaz9120/himawari-rs/compare/v0.35.8...v0.35.9) (2026-08-10)


### その他の変更

* 優等・劣等局面の判定を探索経路内に限定する（ADR-0153） ([#289](https://github.com/kaz9120/himawari-rs/issues/289)) ([ab4bf46](https://github.com/kaz9120/himawari-rs/commit/ab4bf46d3f7aed7329dffe59a2929059a27d9788))

## [0.35.8](https://github.com/kaz9120/himawari-rs/compare/v0.35.7...v0.35.8) (2026-08-09)


### ドキュメント

* ADR-0153のSPRT2走の結果と60+0.6再測定の判断を記録する ([#287](https://github.com/kaz9120/himawari-rs/issues/287)) ([783d4c6](https://github.com/kaz9120/himawari-rs/commit/783d4c6b42a411f14db5aba147b17de9d3d3503a))

## [0.35.7](https://github.com/kaz9120/himawari-rs/compare/v0.35.6...v0.35.7) (2026-08-09)


### 内部

* SPRTの実行・監視・後処理を定型化する（ADR-0154） ([#285](https://github.com/kaz9120/himawari-rs/issues/285)) ([e7f5999](https://github.com/kaz9120/himawari-rs/commit/e7f599983443ac807fd370f2ffdbabef89f32195))

## [0.35.6](https://github.com/kaz9120/himawari-rs/compare/v0.35.5...v0.35.6) (2026-08-09)


### ドキュメント

* 優等局面のroot跨ぎ判定の不具合をADR-0153に起草する ([#283](https://github.com/kaz9120/himawari-rs/issues/283)) ([d123aae](https://github.com/kaz9120/himawari-rs/commit/d123aae5f25c3cc83ed5a5b4eeb6f6c22e8ce525))

## [0.35.5](https://github.com/kaz9120/himawari-rs/compare/v0.35.4...v0.35.5) (2026-08-09)


### 内部

* kifuレポートへ逆転検出と評価推移の表を足す（ADR-0152） ([#281](https://github.com/kaz9120/himawari-rs/issues/281)) ([da9bd68](https://github.com/kaz9120/himawari-rs/commit/da9bd68f86bbdd797ef2e6c074daeb287adc8eae))

## [0.35.4](https://github.com/kaz9120/himawari-rs/compare/v0.35.3...v0.35.4) (2026-08-09)


### その他の変更

* floodgateサイクルの定跡追加へ--depth 28を明示する（ADR-0152） ([#279](https://github.com/kaz9120/himawari-rs/issues/279)) ([a69f984](https://github.com/kaz9120/himawari-rs/commit/a69f984c06874805bcef1c05809542078ca756d6))

## [0.35.3](https://github.com/kaz9120/himawari-rs/compare/v0.35.2...v0.35.3) (2026-08-09)


### 内部

* floodgateの棋譜回収と再解析レポートを実装する（ADR-0152） ([#277](https://github.com/kaz9120/himawari-rs/issues/277)) ([5086dec](https://github.com/kaz9120/himawari-rs/commit/5086decc8e401e473cd3afeaa7826f8135231e8a))

## [0.35.2](https://github.com/kaz9120/himawari-rs/compare/v0.35.1...v0.35.2) (2026-08-09)


### 内部

* 棋譜の局面を定跡へ足すbook seedを実装する（ADR-0152） ([#275](https://github.com/kaz9120/himawari-rs/issues/275)) ([45691eb](https://github.com/kaz9120/himawari-rs/commit/45691eb1baeb7463d0e15ed12032e414d61b7f63))

## [0.35.1](https://github.com/kaz9120/himawari-rs/compare/v0.35.0...v0.35.1) (2026-08-09)


### ドキュメント

* floodgate棋譜の定期回収・分析・定跡追加をADR-0152に起草する ([#273](https://github.com/kaz9120/himawari-rs/issues/273)) ([11d98a1](https://github.com/kaz9120/himawari-rs/commit/11d98a1657313f8526b56ed73c15e8c92637fcf9))

## [0.35.0](https://github.com/kaz9120/himawari-rs/compare/v0.34.23...v0.35.0) (2026-08-09)


### 棋力向上

* 挙動不変の高速化12群を積む（+100.1 Elo、ADR-0151） ([#271](https://github.com/kaz9120/himawari-rs/issues/271)) ([3a60974](https://github.com/kaz9120/himawari-rs/commit/3a6097445514ceea5dba935d68d313bc468e072f))

## [0.34.23](https://github.com/kaz9120/himawari-rs/compare/v0.34.22...v0.34.23) (2026-08-09)


### ドキュメント

* ADR-0151の総括（12群、+22.60% NPS）を記録しROADMAPを更新する ([#269](https://github.com/kaz9120/himawari-rs/issues/269)) ([8b6026c](https://github.com/kaz9120/himawari-rs/commit/8b6026cb009c359c9184e68da2e95e155b102a55))

## [0.34.22](https://github.com/kaz9120/himawari-rs/compare/v0.34.21...v0.34.22) (2026-08-09)


### その他の変更

* check_squaresをノード内の遅延キャッシュにする（ADR-0151群O、+1.82% NPS） ([#267](https://github.com/kaz9120/himawari-rs/issues/267)) ([f1e415d](https://github.com/kaz9120/himawari-rs/commit/f1e415de0198b1ea14335604dab13a4e21f74a3f))

## [0.34.21](https://github.com/kaz9120/himawari-rs/compare/v0.34.20...v0.34.21) (2026-08-09)


### その他の変更

* FT差分適用を両視点1パスにする（ADR-0151群N） ([#265](https://github.com/kaz9120/himawari-rs/issues/265)) ([5abace2](https://github.com/kaz9120/himawari-rs/commit/5abace2a36feb345831a95b7207f79a1aad6057a))

## [0.34.20](https://github.com/kaz9120/himawari-rs/compare/v0.34.19...v0.34.20) (2026-08-09)


### その他の変更

* NNUE第1層をスパース伝播にする（ADR-0151群L、+1.12% NPS） ([#263](https://github.com/kaz9120/himawari-rs/issues/263)) ([6eaeea3](https://github.com/kaz9120/himawari-rs/commit/6eaeea3d99a3ba7102ce85be58e88787ab746267))

## [0.34.19](https://github.com/kaz9120/himawari-rs/compare/v0.34.18...v0.34.19) (2026-08-09)


### ドキュメント

* 部分挿入ソートの二分探索化を棄却する（ADR-0151群M） ([#261](https://github.com/kaz9120/himawari-rs/issues/261)) ([02b6f11](https://github.com/kaz9120/himawari-rs/commit/02b6f111a37a18f7be52b70aa80f9c46650a455b))

## [0.34.18](https://github.com/kaz9120/himawari-rs/compare/v0.34.17...v0.34.18) (2026-08-09)


### ドキュメント

* ADR-0151第3波（群J・G・K）の合算+1.19%を記録する ([#259](https://github.com/kaz9120/himawari-rs/issues/259)) ([eb077b2](https://github.com/kaz9120/himawari-rs/commit/eb077b285622a6780765373927f9f6d51d343b80))

## [0.34.17](https://github.com/kaz9120/himawari-rs/compare/v0.34.16...v0.34.17) (2026-08-09)


### その他の変更

* blockersとpinnersを初回参照時に計算する（ADR-0151群K） ([#257](https://github.com/kaz9120/himawari-rs/issues/257)) ([ebf65bb](https://github.com/kaz9120/himawari-rs/commit/ebf65bbeaee44048791c35de9addce53cabfaa7e))

## [0.34.16](https://github.com/kaz9120/himawari-rs/compare/v0.34.15...v0.34.16) (2026-08-09)


### その他の変更

* BETWEEN/LINE表を方向表とRAY合成へ圧縮する（ADR-0151群G） ([#255](https://github.com/kaz9120/himawari-rs/issues/255)) ([6477903](https://github.com/kaz9120/himawari-rs/commit/6477903a1ee1e6d5d02c6237ad6b08059adec111))

## [0.34.15](https://github.com/kaz9120/himawari-rs/compare/v0.34.14...v0.34.15) (2026-08-09)


### その他の変更

* 探索のPV/NonPVをconst genericsで単相化する（ADR-0151群J） ([#253](https://github.com/kaz9120/himawari-rs/issues/253)) ([9cb1789](https://github.com/kaz9120/himawari-rs/commit/9cb17895a86f8cb7c9726508d5853685fd8f82a0))

## [0.34.14](https://github.com/kaz9120/himawari-rs/compare/v0.34.13...v0.34.14) (2026-08-09)


### ドキュメント

* ADR-0151に第3次プロファイルと群L〜Oを追記する ([#251](https://github.com/kaz9120/himawari-rs/issues/251)) ([d7dc15d](https://github.com/kaz9120/himawari-rs/commit/d7dc15da8a03cfcea0b8ef13591d91becb76bc12))

## [0.34.13](https://github.com/kaz9120/himawari-rs/compare/v0.34.12...v0.34.13) (2026-08-09)


### ドキュメント

* ADR-0151累計のSPRT結果（+60.6 Elo、H1採択）を記録する ([#249](https://github.com/kaz9120/himawari-rs/issues/249)) ([c60199c](https://github.com/kaz9120/himawari-rs/commit/c60199cb155e706c98c5f2ba5e544004b2c09b47))

## [0.34.12](https://github.com/kaz9120/himawari-rs/compare/v0.34.11...v0.34.12) (2026-08-09)


### 内部

* 配布バイナリをPGOで作る（ADR-0151群I） ([#247](https://github.com/kaz9120/himawari-rs/issues/247)) ([868f1dc](https://github.com/kaz9120/himawari-rs/commit/868f1dc8e1cf244af9f001b7f0ec16a5868e9153))

## [0.34.11](https://github.com/kaz9120/himawari-rs/compare/v0.34.10...v0.34.11) (2026-08-09)


### その他の変更

* 二歩マスクのビット演算化とSEEの最安駒選択の表引き化（ADR-0151群D） ([#245](https://github.com/kaz9120/himawari-rs/issues/245)) ([cae01f0](https://github.com/kaz9120/himawari-rs/commit/cae01f0046368bb3d65b6acc6a99df8340891e34))

## [0.34.10](https://github.com/kaz9120/himawari-rs/compare/v0.34.9...v0.34.10) (2026-08-09)


### その他の変更

* NNUE隠れ層の行束ねを8行にしdotへ専用命令を使う（ADR-0151群C、+1.01% NPS） ([#243](https://github.com/kaz9120/himawari-rs/issues/243)) ([cdf998b](https://github.com/kaz9120/himawari-rs/commit/cdf998b2e0041f29296a61481b73bb783b704f7c))

## [0.34.9](https://github.com/kaz9120/himawari-rs/compare/v0.34.8...v0.34.9) (2026-08-09)


### その他の変更

* accumulatorのアラインとリダクション表の縮小（ADR-0151群H） ([#241](https://github.com/kaz9120/himawari-rs/issues/241)) ([4b4e7c9](https://github.com/kaz9120/himawari-rs/commit/4b4e7c954247d021b2b38e2f516ebf6a5d8ac655))

## [0.34.8](https://github.com/kaz9120/himawari-rs/compare/v0.34.7...v0.34.8) (2026-08-09)


### 内部

* PGOビルドの手順をスクリプトに固定する（ADR-0151群I、+10.67% NPS） ([#239](https://github.com/kaz9120/himawari-rs/issues/239)) ([8699a85](https://github.com/kaz9120/himawari-rs/commit/8699a852e53305b6c90d070b1655393ad786f8a1))

## [0.34.7](https://github.com/kaz9120/himawari-rs/compare/v0.34.6...v0.34.7) (2026-08-09)


### その他の変更

* 複合ビットボードで利きの合成を差分維持する（ADR-0151群F、+2.47% NPS） ([#237](https://github.com/kaz9120/himawari-rs/issues/237)) ([e9d7579](https://github.com/kaz9120/himawari-rs/commit/e9d75799fa0478dc7c90df3406a5c3b8be67f9b2))

## [0.34.6](https://github.com/kaz9120/himawari-rs/compare/v0.34.5...v0.34.6) (2026-08-09)


### ドキュメント

* ADR-0151の群A・B合算（+13.79% NPS）を記録しROADMAPを更新する ([#235](https://github.com/kaz9120/himawari-rs/issues/235)) ([d6f7400](https://github.com/kaz9120/himawari-rs/commit/d6f74007d30a5dd8a1a843ae7748ee6491538e04))

## [0.34.5](https://github.com/kaz9120/himawari-rs/compare/v0.34.4...v0.34.5) (2026-08-09)


### その他の変更

* NNUEのFT差分適用を1パスに融合する（ADR-0151群A、+9.20% NPS） ([#232](https://github.com/kaz9120/himawari-rs/issues/232)) ([0cda1b9](https://github.com/kaz9120/himawari-rs/commit/0cda1b9bf32734f8e7474b3c473001d3e3817abd))
* 探索ホットパスのヒープ確保を消す（ADR-0151群B、+5.02% NPS） ([#234](https://github.com/kaz9120/himawari-rs/issues/234)) ([b196ea6](https://github.com/kaz9120/himawari-rs/commit/b196ea64f42806e13b0a68511fb3f96d43ed0c55))

## [0.34.4](https://github.com/kaz9120/himawari-rs/compare/v0.34.3...v0.34.4) (2026-08-09)


### ドキュメント

* ADR-0151に群F〜K（複合ビットボード・PGOほか）を追記する ([#230](https://github.com/kaz9120/himawari-rs/issues/230)) ([675ef5b](https://github.com/kaz9120/himawari-rs/commit/675ef5bb1bacbabacc9a5749e687a7c16f39af6e))

## [0.34.3](https://github.com/kaz9120/himawari-rs/compare/v0.34.2...v0.34.3) (2026-08-09)


### ドキュメント

* 挙動を変えない高速化の第2弾を洗い出す（ADR-0151） ([#228](https://github.com/kaz9120/himawari-rs/issues/228)) ([cfb8eb7](https://github.com/kaz9120/himawari-rs/commit/cfb8eb77336e1009fc36fc88e5e97be2b8ff732d))

## [0.34.2](https://github.com/kaz9120/himawari-rs/compare/v0.34.1...v0.34.2) (2026-08-09)


### ドキュメント

* 世代1の棄却を記録し、ADR-0150の検証損失の扱いを直す ([#226](https://github.com/kaz9120/himawari-rs/issues/226)) ([7a4d775](https://github.com/kaz9120/himawari-rs/commit/7a4d775c8f616c4f8cec094d13d04c719ce787e7))

## [0.34.1](https://github.com/kaz9120/himawari-rs/compare/v0.34.0...v0.34.1) (2026-08-09)


### その他の変更

* RootStrapの評価方法を決め直し、生成の終盤欠落を直す（ADR-0144・0145・0150） ([#224](https://github.com/kaz9120/himawari-rs/issues/224)) ([aafc57d](https://github.com/kaz9120/himawari-rs/commit/aafc57debfb7552c32ec83c39122a928a2aab929))

## [0.34.0](https://github.com/kaz9120/himawari-rs/compare/v0.33.2...v0.34.0) (2026-08-08)


### 棋力向上

* gensfenを並列化し、記録する局面を静止局面にする（ADR-0144・0136） ([#221](https://github.com/kaz9120/himawari-rs/issues/221)) ([0d936a4](https://github.com/kaz9120/himawari-rs/commit/0d936a4e03ddf73907dcba0ed4514cc090cef265))

## [0.33.2](https://github.com/kaz9120/himawari-rs/compare/v0.33.1...v0.33.2) (2026-08-08)


### その他の変更

* 定跡の網羅率を数え、利きテーブルの常時更新を見送る（ADR-0146・0148） ([#220](https://github.com/kaz9120/himawari-rs/issues/220)) ([3158893](https://github.com/kaz9120/himawari-rs/commit/315889303ee0921f78a90ca29f4771628bb080f0))

## [0.33.1](https://github.com/kaz9120/himawari-rs/compare/v0.33.0...v0.33.1) (2026-08-08)


### 内部

* 実験の実行とログを規約で固定する（ADR-0149） ([#217](https://github.com/kaz9120/himawari-rs/issues/217)) ([389ec2b](https://github.com/kaz9120/himawari-rs/commit/389ec2bc5af0159014747dbb9f77c0176bb5bdee))

## [0.33.0](https://github.com/kaz9120/himawari-rs/compare/v0.32.0...v0.33.0) (2026-08-08)


### 棋力向上

* 盤面の利きを差分で持つEffectTableを置く（ADR-0148） ([#216](https://github.com/kaz9120/himawari-rs/issues/216)) ([8d375e9](https://github.com/kaz9120/himawari-rs/commit/8d375e991a0baabf68c5c8f2011ae5098fa8235f))

## [0.32.0](https://github.com/kaz9120/himawari-rs/compare/v0.31.2...v0.32.0) (2026-08-08)


### 棋力向上

* 教師データの世代ループに要る2つの部品を置き、EffectBucketを起草する（ADR-0144・0145・0147） ([#214](https://github.com/kaz9120/himawari-rs/issues/214)) ([e8307ca](https://github.com/kaz9120/himawari-rs/commit/e8307caa1a735aeb632fbfa6e5c7ef8bfc3d1528))

## [0.31.2](https://github.com/kaz9120/himawari-rs/compare/v0.31.1...v0.31.2) (2026-08-08)


### その他の変更

* 定跡の浅い層を全合法手で埋め、最新ネットで引き直せるようにする（ADR-0146） ([#212](https://github.com/kaz9120/himawari-rs/issues/212)) ([59b4027](https://github.com/kaz9120/himawari-rs/commit/59b4027818aa73263261e0e7731faf66e19ea417))

## [0.31.1](https://github.com/kaz9120/himawari-rs/compare/v0.31.0...v0.31.1) (2026-08-08)


### その他の変更

* release-net.shがフォーマット版5・6のネットを読めないのを直す ([#210](https://github.com/kaz9120/himawari-rs/issues/210)) ([a1f8137](https://github.com/kaz9120/himawari-rs/commit/a1f8137b95259efc317cecf1500a35561cd69c5e))

## [0.31.0](https://github.com/kaz9120/himawari-rs/compare/v0.30.1...v0.31.0) (2026-08-08)


### 棋力向上

* 教師局面を静止化したネットを現行構成にする（+13.9 Elo、ADR-0136） ([#208](https://github.com/kaz9120/himawari-rs/issues/208)) ([71cece8](https://github.com/kaz9120/himawari-rs/commit/71cece8f004f44248f41f36c4d4769306668839b))

## [0.30.1](https://github.com/kaz9120/himawari-rs/compare/v0.30.0...v0.30.1) (2026-08-07)


### その他の変更

* psv quietの符号バグを直し、教師局面の静止化を3億局面で測る（ADR-0136） ([#206](https://github.com/kaz9120/himawari-rs/issues/206)) ([37ecc24](https://github.com/kaz9120/himawari-rs/commit/37ecc24285db7df7123d8b8b673470729abb9e99))

## [0.30.0](https://github.com/kaz9120/himawari-rs/compare/v0.29.0...v0.30.0) (2026-08-06)


### 棋力向上

* FT重みi8ビルドとクリップ済みネットを現行構成にする（+29.8 Elo、ADR-0138） ([#204](https://github.com/kaz9120/himawari-rs/issues/204)) ([6408d37](https://github.com/kaz9120/himawari-rs/commit/6408d379e2529f5fd03026c14420cb175c211437))

## [0.29.0](https://github.com/kaz9120/himawari-rs/compare/v0.28.3...v0.29.0) (2026-08-05)


### 棋力向上

* FT重みのi8格納と、教師局面の静止化ツール（ADR-0138・0136） ([#202](https://github.com/kaz9120/himawari-rs/issues/202)) ([c166c9a](https://github.com/kaz9120/himawari-rs/commit/c166c9abb86aaa772586030b801c103836225165))

## [0.28.3](https://github.com/kaz9120/himawari-rs/compare/v0.28.2...v0.28.3) (2026-08-05)


### 内部

* FT重みのクリップ制約と、量子化スケールのリーグ戦（ADR-0138） ([#200](https://github.com/kaz9120/himawari-rs/issues/200)) ([bb452c6](https://github.com/kaz9120/himawari-rs/commit/bb452c61aca2e0176a0563a4010fde691a94a908))

## [0.28.2](https://github.com/kaz9120/himawari-rs/compare/v0.28.1...v0.28.2) (2026-08-05)


### ドキュメント

* ADR-0141をrejectedで閉じ、多段延長を棄却へ格上げする ([#198](https://github.com/kaz9120/himawari-rs/issues/198)) ([07040ac](https://github.com/kaz9120/himawari-rs/commit/07040ac2c9b73b9e5dadd28f64207131b471cd40))

## [0.28.1](https://github.com/kaz9120/himawari-rs/compare/v0.28.0...v0.28.1) (2026-08-05)


### その他の変更

* env.shの既定の評価関数を現行のhalfkp_2990M_factへ揃える ([#196](https://github.com/kaz9120/himawari-rs/issues/196)) ([9ec4d92](https://github.com/kaz9120/himawari-rs/commit/9ec4d92c42427e374b031d75967217d99e87749d))

## [0.28.0](https://github.com/kaz9120/himawari-rs/compare/v0.27.21...v0.28.0) (2026-08-05)


### 棋力向上

* 教師データを29.9億へ広げたネットを採用する（+24.8 Elo、ADR-0135） ([#193](https://github.com/kaz9120/himawari-rs/issues/193)) ([d69df91](https://github.com/kaz9120/himawari-rs/commit/d69df91f23469dd9c1f8588cd2c2d72b74a46453))


### その他の変更

* release-net.shが版3以降のネットの学習来歴を読めないのを直す ([#194](https://github.com/kaz9120/himawari-rs/issues/194)) ([aec29b3](https://github.com/kaz9120/himawari-rs/commit/aec29b316c70558592e169e3f447d36ecbcac5c9))

## [0.27.21](https://github.com/kaz9120/himawari-rs/compare/v0.27.20...v0.27.21) (2026-08-04)


### ドキュメント

* 優勢時の頓死の観察をROADMAPの候補へ記録する ([#191](https://github.com/kaz9120/himawari-rs/issues/191)) ([fb029da](https://github.com/kaz9120/himawari-rs/commit/fb029da66b3a3f78b7020eecb2d361dcc7d42191))

## [0.27.20](https://github.com/kaz9120/himawari-rs/compare/v0.27.19...v0.27.20) (2026-08-04)


### ドキュメント

* ADR-0139をrejectedで閉じる ([#189](https://github.com/kaz9120/himawari-rs/issues/189)) ([6c24a11](https://github.com/kaz9120/himawari-rs/commit/6c24a1133148ff2a8a1bda4c26726131f931d206))

## [0.27.19](https://github.com/kaz9120/himawari-rs/compare/v0.27.18...v0.27.19) (2026-08-04)


### 内部

* hao_depth9の第3グループを取得し、教師データを30.0億へ広げる（ADR-0135） ([#187](https://github.com/kaz9120/himawari-rs/issues/187)) ([6096f29](https://github.com/kaz9120/himawari-rs/commit/6096f295a4f2ba5cf56f5f802d1106d0d4c64f64))

## [0.27.18](https://github.com/kaz9120/himawari-rs/compare/v0.27.17...v0.27.18) (2026-08-04)


### ドキュメント

* 棋力向上アイデア10件のADRを起草する（ADR-0135〜0144） ([#185](https://github.com/kaz9120/himawari-rs/issues/185)) ([4660f50](https://github.com/kaz9120/himawari-rs/commit/4660f504c2445818f4900ad3a09ddaca1ad20bf7))

## [0.27.17](https://github.com/kaz9120/himawari-rs/compare/v0.27.16...v0.27.17) (2026-08-04)


### 内部

* 利き予測の事前学習と、後段容量の測定（ADR-0133・0134） ([#183](https://github.com/kaz9120/himawari-rs/issues/183)) ([ebc0b5d](https://github.com/kaz9120/himawari-rs/commit/ebc0b5d3f9db82dc5c6521b74b0c1b3e93379a94))

## [0.27.16](https://github.com/kaz9120/himawari-rs/compare/v0.27.15...v0.27.16) (2026-08-03)


### 内部

* FT表現学習の測定基盤と、256x16の棄却（ADR-0131・0132・0133） ([#181](https://github.com/kaz9120/himawari-rs/issues/181)) ([f876075](https://github.com/kaz9120/himawari-rs/commit/f8760752b678fdef6aa1bf7616459a95681c775c))

## [0.27.15](https://github.com/kaz9120/himawari-rs/compare/v0.27.14...v0.27.15) (2026-08-02)


### 内部

* FTの表現を厚くする試みと、後段を削って速くする結果（ADR-0127・0129・0130） ([#179](https://github.com/kaz9120/himawari-rs/issues/179)) ([139e5f6](https://github.com/kaz9120/himawari-rs/commit/139e5f6081b23aeedec8654e77eb3e5e96f6adad))

## [0.27.14](https://github.com/kaz9120/himawari-rs/compare/v0.27.13...v0.27.14) (2026-08-02)


### 内部

* ネットワーク構成8種を測り、現行構成の維持を決める（ADR-0127） ([#175](https://github.com/kaz9120/himawari-rs/issues/175)) ([ba75220](https://github.com/kaz9120/himawari-rs/commit/ba75220bef2f785ceb9f08b24634c8b24a5eae86))

## [0.27.13](https://github.com/kaz9120/himawari-rs/compare/v0.27.12...v0.27.13) (2026-08-02)


### 内部

* 総当たりリーグ戦で複数候補の順位を出せるようにする（ADR-0128） ([#176](https://github.com/kaz9120/himawari-rs/issues/176)) ([8dca05f](https://github.com/kaz9120/himawari-rs/commit/8dca05f2d9891b52e70beb203648dab6cadf45e8))

## [0.27.12](https://github.com/kaz9120/himawari-rs/compare/v0.27.11...v0.27.12) (2026-08-01)


### ドキュメント

* ネットワーク構造の探索を学習前のNPS計測から始める（ADR-0127） ([#173](https://github.com/kaz9120/himawari-rs/issues/173)) ([45f701b](https://github.com/kaz9120/himawari-rs/commit/45f701bbf7532c8a43f9b457bdd3512efc57da87))

## [0.27.11](https://github.com/kaz9120/himawari-rs/compare/v0.27.10...v0.27.11) (2026-08-01)


### その他の変更

* 教師信号の詰みスコアを測り、現行の素通しを維持する（ADR-0126） ([#171](https://github.com/kaz9120/himawari-rs/issues/171)) ([28e8f9f](https://github.com/kaz9120/himawari-rs/commit/28e8f9f575ae02ea2c1bafbb206f4dda63fb20da))

## [0.27.10](https://github.com/kaz9120/himawari-rs/compare/v0.27.9...v0.27.10) (2026-08-01)


### その他の変更

* 使われていないコードを消す ([#169](https://github.com/kaz9120/himawari-rs/issues/169)) ([45f5a17](https://github.com/kaz9120/himawari-rs/commit/45f5a17b2e1ae68972414e1b167fb06edc1973a0))

## [0.27.9](https://github.com/kaz9120/himawari-rs/compare/v0.27.8...v0.27.9) (2026-08-01)


### その他の変更

* Rust版の学習器を削除する（ADR-0039をsuperseded） ([#167](https://github.com/kaz9120/himawari-rs/issues/167)) ([cf6bec5](https://github.com/kaz9120/himawari-rs/commit/cf6bec528d5ac465001e3ba715c669c0d580f00a))

## [0.27.8](https://github.com/kaz9120/himawari-rs/compare/v0.27.7...v0.27.8) (2026-08-01)


### ドキュメント

* 整理と高速化の一巡をROADMAPへ反映する ([#165](https://github.com/kaz9120/himawari-rs/issues/165)) ([a921f8a](https://github.com/kaz9120/himawari-rs/commit/a921f8a70439e8b9d3046dfc5b6d9d40ab9ba35e))

## [0.27.7](https://github.com/kaz9120/himawari-rs/compare/v0.27.6...v0.27.7) (2026-08-01)


### その他の変更

* ホットパスのコピーと間接参照を削る（ADR-0124、第2・3群、+2.71% NPS） ([#163](https://github.com/kaz9120/himawari-rs/issues/163)) ([1863359](https://github.com/kaz9120/himawari-rs/commit/18633593f25c095a42a5e99efc669849d9edd90f))

## [0.27.6](https://github.com/kaz9120/himawari-rs/compare/v0.27.5...v0.27.6) (2026-08-01)


### その他の変更

* 探索本体を責務ごとに切り出す（ADR-0125） ([#160](https://github.com/kaz9120/himawari-rs/issues/160)) ([3ef7302](https://github.com/kaz9120/himawari-rs/commit/3ef7302e42bea55f69746f6fd5ddb81f9a368d05))

## [0.27.5](https://github.com/kaz9120/himawari-rs/compare/v0.27.4...v0.27.5) (2026-08-01)


### その他の変更

* リリースの作成を既定でやめ、--applyで明示させる（ADR-0122） ([#159](https://github.com/kaz9120/himawari-rs/issues/159)) ([5028f77](https://github.com/kaz9120/himawari-rs/commit/5028f77361635105e182a59be955708882ecaea7))

## [0.27.4](https://github.com/kaz9120/himawari-rs/compare/v0.27.3...v0.27.4) (2026-08-01)


### その他の変更

* NNUEの全計算でヒープを確保しないようにする（ADR-0124、第1群） ([#156](https://github.com/kaz9120/himawari-rs/issues/156)) ([d46272a](https://github.com/kaz9120/himawari-rs/commit/d46272adc27223ef5c756598c1991dcbbfd1f835))

## [0.27.3](https://github.com/kaz9120/himawari-rs/compare/v0.27.2...v0.27.3) (2026-08-01)


### その他の変更

* 長時間走る処理を停止ファイルで止められるようにする（ADR-0123） ([#155](https://github.com/kaz9120/himawari-rs/issues/155)) ([676b75f](https://github.com/kaz9120/himawari-rs/commit/676b75f8edd63a670c7c3bf082f857480b9e3cd8))

## [0.27.2](https://github.com/kaz9120/himawari-rs/compare/v0.27.1...v0.27.2) (2026-08-01)


### 内部

* 開発スクリプトを役割で3言語に分ける（ADR-0122） ([#153](https://github.com/kaz9120/himawari-rs/issues/153)) ([1b3b597](https://github.com/kaz9120/himawari-rs/commit/1b3b59727ff41761c69d845868987371cd32a9ef))

## [0.27.1](https://github.com/kaz9120/himawari-rs/compare/v0.27.0...v0.27.1) (2026-08-01)


### その他の変更

* 定跡を損失の小さい順に掘り、上限と再開を付ける（ADR-0121） ([#151](https://github.com/kaz9120/himawari-rs/issues/151)) ([b0d2f78](https://github.com/kaz9120/himawari-rs/commit/b0d2f786f31385189052497eb50ca52fe7684e97))

## [0.27.0](https://github.com/kaz9120/himawari-rs/compare/v0.26.0...v0.27.0) (2026-07-31)


### 棋力向上

* 定跡・投票・実務オプションを参照実装へ揃える（G10、ADR-0119） ([75fa045](https://github.com/kaz9120/himawari-rs/commit/75fa045f92b1cc113b96f1d5cc825560a08cf45b))

## [0.26.0](https://github.com/kaz9120/himawari-rs/compare/v0.25.0...v0.26.0) (2026-07-31)


### 棋力向上

* 反復深化とaspirationを参照実装へ揃える（+55.6 Elo、G9、ADR-0118） ([89ecaf9](https://github.com/kaz9120/himawari-rs/commit/89ecaf9a47b4d3948826ee89a637e067a4c97ba9))

## [0.25.0](https://github.com/kaz9120/himawari-rs/compare/v0.24.0...v0.25.0) (2026-07-31)


### 棋力向上

* ponderの会計・継続・予約を参照実装へ揃える（+19.3 Elo、G8、ADR-0117） ([9c054f2](https://github.com/kaz9120/himawari-rs/commit/9c054f2e8c24965bfd5d88e3e26cc6a3a0b06dea))

## [0.24.0](https://github.com/kaz9120/himawari-rs/compare/v0.23.0...v0.24.0) (2026-07-30)


### 棋力向上

* 停止を予約する構造へ移し最小思考時間を入れる（G7、ADR-0116） ([5d165b3](https://github.com/kaz9120/himawari-rs/commit/5d165b3f824e36c7bb966cf01b4cb2d2829cc904))

## [0.23.0](https://github.com/kaz9120/himawari-rs/compare/v0.22.0...v0.23.0) (2026-07-30)


### 棋力向上

* qsearchを参照実装へ揃えmate_1plyを指さない方式へ書き換える（+45.6 Elo、G6、ADR-0115） ([3719bfe](https://github.com/kaz9120/himawari-rs/commit/3719bfe6b6347729977c5c1ef29e8178724c9cf0))

## [0.22.0](https://github.com/kaz9120/himawari-rs/compare/v0.21.0...v0.22.0) (2026-07-30)


### 棋力向上

* singularの条件とmulti-cut・negative extensionを参照実装へ揃える（+48.2 Elo、G5、ADR-0114） ([8c501ea](https://github.com/kaz9120/himawari-rs/commit/8c501ea741d28a0af0b17b477f33d92257f5899b))

## [0.21.0](https://github.com/kaz9120/himawari-rs/compare/v0.20.0...v0.21.0) (2026-07-30)


### 棋力向上

* improvingの再定義とevalベース枝刈りを参照実装へ揃える（+41.1 Elo、G4、ADR-0113） ([6aa0013](https://github.com/kaz9120/himawari-rs/commit/6aa001302be80e320e3779de53904268c01790ab))

## [0.20.0](https://github.com/kaz9120/himawari-rs/compare/v0.19.0...v0.20.0) (2026-07-29)


### 棋力向上

* ムーブループの枝刈りを参照実装へ揃える（+95.2 Elo、G3、ADR-0112） ([13dbef9](https://github.com/kaz9120/himawari-rs/commit/13dbef9433ebcf188181c2f6032c8ad4710b4f23))

## [0.19.0](https://github.com/kaz9120/himawari-rs/compare/v0.18.0...v0.19.0) (2026-07-29)


### 棋力向上

* statScoreとLMRのリダクションを参照実装へ揃える（+124.0 Elo、G2、ADR-0111） ([46b8378](https://github.com/kaz9120/himawari-rs/commit/46b8378a2242a1756bf59b4b822f53071b7bbdc8))

## [0.18.0](https://github.com/kaz9120/himawari-rs/compare/v0.17.0...v0.18.0) (2026-07-29)


### 棋力向上

* historyの面と更新を参照実装へ揃える（+88.5 Elo、G1、ADR-0110） ([97ccaa4](https://github.com/kaz9120/himawari-rs/commit/97ccaa48d6c776a023bfe357c330cf620aac9d31))

## [0.17.0](https://github.com/kaz9120/himawari-rs/compare/v0.16.3...v0.17.0) (2026-07-29)


### 棋力向上

* correction historyを3系統に増やす（+17.7 Elo、ADR-0085） ([#56](https://github.com/kaz9120/himawari-rs/issues/56)) ([2d2709e](https://github.com/kaz9120/himawari-rs/commit/2d2709ea2b2595cea6bbed8d0e1fa7b38c685b6c))
* history bonus/malus式を再設計する（+42.2 Elo、ADR-0073） ([#16](https://github.com/kaz9120/himawari-rs/issues/16)) ([12a9ee1](https://github.com/kaz9120/himawari-rs/commit/12a9ee123496ac7d625314e4906c36c8bde47253))
* lmrDepth基準を導入しSEEベースの枝刈りを入れる（+45.6 Elo、ADR-0090） ([#54](https://github.com/kaz9120/himawari-rs/issues/54)) ([ed1eeeb](https://github.com/kaz9120/himawari-rs/commit/ed1eeeb7aefd7dfb3b8112f55dac78070d02529e))
* NNUE隠れ層の内積を専用命令で計算する（+59.7 Elo、ADR-0099） ([#88](https://github.com/kaz9120/himawari-rs/issues/88)) ([a67c2c5](https://github.com/kaz9120/himawari-rs/commit/a67c2c5728a9783070cfdb44fedca05824f77b3b))
* SEEを駒打ちに対応させる（+67.0 Elo、ADR-0091） ([#59](https://github.com/kaz9120/himawari-rs/issues/59)) ([e578957](https://github.com/kaz9120/himawari-rs/commit/e578957b587db00e8bdb62d4ec947d75c4ad20e1))
* USIの入出力をファイルへ写すDebugLogFileを足す ([#117](https://github.com/kaz9120/himawari-rs/issues/117)) ([0eea97c](https://github.com/kaz9120/himawari-rs/commit/0eea97c629975f2389dbca4b24d6047996ec3e0f))
* 指し手の最大スコア探索をSoA＋SIMDにする（+106.7 Elo、ADR-0100） ([#89](https://github.com/kaz9120/himawari-rs/issues/89)) ([794895a](https://github.com/kaz9120/himawari-rs/commit/794895a53f83edb8853b7d105d2a6485b0735f68))
* 置換表の下界による簡易ProbCutを入れる（+15.6 Elo、ADR-0078） ([#31](https://github.com/kaz9120/himawari-rs/issues/31)) ([42e97f9](https://github.com/kaz9120/himawari-rs/commit/42e97f977e640ac3e034ec5b2f8669db3184b134))
* 静止探索にfutility枝刈りを入れる（+57.3 Elo、ADR-0077） ([#28](https://github.com/kaz9120/himawari-rs/issues/28)) ([02bc6e7](https://github.com/kaz9120/himawari-rs/commit/02bc6e76cebc290b0b585af9c26da4dd4be9297e))


### その他の変更

* aspirationのfail high/lowをinfoで報告する（ADR-0092） ([#58](https://github.com/kaz9120/himawari-rs/issues/58)) ([6d51068](https://github.com/kaz9120/himawari-rs/commit/6d5106837e8535d4594cc238793092151fce17a7))
* LMRのリダクションを1024倍固定小数にする（ADR-0076） ([#22](https://github.com/kaz9120/himawari-rs/issues/22)) ([e732914](https://github.com/kaz9120/himawari-rs/commit/e7329145b2b4852968c29ed05068ca6389472827))
* mate_1plyの検証を軽くする（ADR-0094） ([#70](https://github.com/kaz9120/himawari-rs/issues/70)) ([6fd85c4](https://github.com/kaz9120/himawari-rs/commit/6fd85c435d78d562a0cc7a8c5b4e62eee689ab75))
* MoveListのゼロ埋めをやめる（ADR-0101） ([#92](https://github.com/kaz9120/himawari-rs/issues/92)) ([4723258](https://github.com/kaz9120/himawari-rs/commit/47232587944ff9250b6b55e7b3c3e99ad6233388))
* MultiPVの出力スコアを直前ラインの出力値で頭打ちにする ([#43](https://github.com/kaz9120/himawari-rs/issues/43)) ([2acf78e](https://github.com/kaz9120/himawari-rs/commit/2acf78ea499ac620b721d53c4cda6d02a587f89d))
* MultiPVの出力をスコア降順に整える（ADR-0032） ([#24](https://github.com/kaz9120/himawari-rs/issues/24)) ([15380d6](https://github.com/kaz9120/himawari-rs/commit/15380d6b583e73e9ef8c7dad83fb63def7551232))
* MultiPVの降順保証を並べ替えから出力スコアの頭打ちへ変える ([#26](https://github.com/kaz9120/himawari-rs/issues/26)) ([eee081f](https://github.com/kaz9120/himawari-rs/commit/eee081f9c2054d6926282845751cb66584268057))
* plyごとの状態をStackへ集約しcutNodeを配線する（G0、ADR-0109） ([931b559](https://github.com/kaz9120/himawari-rs/commit/931b5593b4231c55c249730e66507c8bf466d87f))
* SEEで初手の成りを扱う（ADR-0095） ([#72](https://github.com/kaz9120/himawari-rs/issues/72)) ([9535517](https://github.com/kaz9120/himawari-rs/commit/953551775143a75ebb509b65998a79f729e1a420))
* setoptionの値を元の行から切り出し、引用符を落とす ([#40](https://github.com/kaz9120/himawari-rs/issues/40)) ([3cb18b1](https://github.com/kaz9120/himawari-rs/commit/3cb18b1d58e6f7b888736fbec2b401b7340b0058))
* USIのinfoにseldepthとcurrmoveを出す（ADR-0086） ([#48](https://github.com/kaz9120/himawari-rs/issues/48)) ([fde0fa8](https://github.com/kaz9120/himawari-rs/commit/fde0fa893bde91f9c72163f547638b6197befab1))
* WindowsバイナリをMSVCランタイム静的リンクで配布する（ADR-0083） ([#36](https://github.com/kaz9120/himawari-rs/issues/36)) ([f6684ff](https://github.com/kaz9120/himawari-rs/commit/f6684ff98e7be811cd4fc0ceb8d9670f036f034d))
* 勝ちの詰みを見つけたら反復深化を打ち切る（ADR-0088） ([#51](https://github.com/kaz9120/himawari-rs/issues/51)) ([f6f97d6](https://github.com/kaz9120/himawari-rs/commit/f6f97d61e083376e5319432bd874b6b0cce54bac))
* 打ち切り時に未確定のaspiration窓外れを最後に残さない ([#116](https://github.com/kaz9120/himawari-rs/issues/116)) ([94bd980](https://github.com/kaz9120/himawari-rs/commit/94bd98078eaeed4dc8bde9e0d81fa7ecfeceea44))
* 新規開始のsprt-run.shが空配列の展開で落ちるのを直す ([#93](https://github.com/kaz9120/himawari-rs/issues/93)) ([ba6ade7](https://github.com/kaz9120/himawari-rs/commit/ba6ade76913072155978fc91c0d92850f71e683a))
* 深さ1のイテレーションを終えるまでstopを無視する ([#101](https://github.com/kaz9120/himawari-rs/issues/101)) ([4446c8d](https://github.com/kaz9120/himawari-rs/commit/4446c8d02b91cd83cc8717623b560358344a0382))
* 詰まされる側でも反復深化を打ち切る（ADR-0088） ([#66](https://github.com/kaz9120/himawari-rs/issues/66)) ([649586c](https://github.com/kaz9120/himawari-rs/commit/649586c2265e1da254d52f95892c912de9bf1ccc))


### ドキュメント

* 2026-07-29の時間管理キャンペーンをROADMAPに残す ([#112](https://github.com/kaz9120/himawari-rs/issues/112)) ([638cc4f](https://github.com/kaz9120/himawari-rs/commit/638cc4fa97e24ab3c27cc0398cd82c053b4bb1c7))
* ADR-0071の記述を実装結果に合わせる ([#6](https://github.com/kaz9120/himawari-rs/issues/6)) ([f19df6d](https://github.com/kaz9120/himawari-rs/commit/f19df6d8b270006783dde3e2e4ca1bbdcca978b7))
* aspirationの窓外れが評価値とともに増えることを記録する ([#122](https://github.com/kaz9120/himawari-rs/issues/122)) ([6e7a134](https://github.com/kaz9120/himawari-rs/commit/6e7a13474dc258ec86f94fef9fdbc1b2b5bf37fc))
* book-v1の公開を記録しADR-0082の判断を実際に合わせる ([#38](https://github.com/kaz9120/himawari-rs/issues/38)) ([8b40611](https://github.com/kaz9120/himawari-rs/commit/8b406119c8ac56088a4790b57143d67b06d4f4f4))
* capture historyの再挑戦を棄却として記録する（ADR-0097） ([#84](https://github.com/kaz9120/himawari-rs/issues/84)) ([2b18c0c](https://github.com/kaz9120/himawari-rs/commit/2b18c0c2a32c0e2632805ae1a1f1a5d0c76309e9))
* floodgateの対局から入玉局面の評価という論点を立てる ([#119](https://github.com/kaz9120/himawari-rs/issues/119)) ([18eac68](https://github.com/kaz9120/himawari-rs/commit/18eac68a3a64448e57e492e9948235af3d63b372))
* G0の範囲をStackの器に絞り、cutNodeの実引数表を足す（ADR-0109） ([2c40e76](https://github.com/kaz9120/himawari-rs/commit/2c40e7628c681e9511a8e8460625c4d7d42583eb))
* history pruningの不発を診断し、bonus/malus式の再設計を起草する ([#12](https://github.com/kaz9120/himawari-rs/issues/12)) ([3092d3d](https://github.com/kaz9120/himawari-rs/commit/3092d3ddcaf12a909f589df882ee22212b329f20))
* history pruningの再挑戦を280局で打ち切る（ADR-0072） ([#18](https://github.com/kaz9120/himawari-rs/issues/18)) ([e15388f](https://github.com/kaz9120/himawari-rs/commit/e15388f01661256c8cfca68b5f9c7db9f3a308fd))
* LMRのcutNode項を棄却として記録する（ADR-0084） ([#46](https://github.com/kaz9120/himawari-rs/issues/46)) ([23af0b7](https://github.com/kaz9120/himawari-rs/commit/23af0b71d1b1e3db39591289645ab021d0f46aeb))
* ponderhitでの探索継続を棄却した記録を残す（ADR-0106） ([#113](https://github.com/kaz9120/himawari-rs/issues/113)) ([07dd208](https://github.com/kaz9120/himawari-rs/commit/07dd20811a6aba80fd3c80f1db59e4d3e5cef4fe))
* ponderの時間会計を棄却した記録を残す（ADR-0104） ([#107](https://github.com/kaz9120/himawari-rs/issues/107)) ([262fc06](https://github.com/kaz9120/himawari-rs/commit/262fc0679d488e26c97fa2d0decbf877f6f5b802))
* ponder有効時の思考時間1.25倍を棄却した記録を残す（ADR-0107） ([#121](https://github.com/kaz9120/himawari-rs/issues/121)) ([bd6179f](https://github.com/kaz9120/himawari-rs/commit/bd6179f20b6d909f04a473d4d0571e33b9b2ef09))
* razoringのマージン見直しを機能検証で棄却する（ADR-0075） ([#20](https://github.com/kaz9120/himawari-rs/issues/20)) ([686bb4c](https://github.com/kaz9120/himawari-rs/commit/686bb4cd5ac4e7c4a49c6cb6b0d4aec9f39d521c))
* RFPのマージン緩和を棄却として記録する（ADR-0096） ([#74](https://github.com/kaz9120/himawari-rs/issues/74)) ([0a01b0a](https://github.com/kaz9120/himawari-rs/commit/0a01b0ab09ad53630baedc3c6a25aaf57c131230))
* ROADMAPの現在地を2026-07-29時点へ更新する ([#62](https://github.com/kaz9120/himawari-rs/issues/62)) ([00b0c35](https://github.com/kaz9120/himawari-rs/commit/00b0c35f29b752145eb763c3c195dc39f377d2c7))
* rootの1位2位差を判定材料に足す案を実装前に棄却する（ADR-0103） ([#102](https://github.com/kaz9120/himawari-rs/issues/102)) ([217fe96](https://github.com/kaz9120/himawari-rs/commit/217fe963f4c39e7957ad047ec67cda55beeba151))
* SPRTの前に機能検証を行う規約を定める（ADR-0074） ([#14](https://github.com/kaz9120/himawari-rs/issues/14)) ([830aff5](https://github.com/kaz9120/himawari-rs/commit/830aff58d643e9114ca08aa0bc04b5d96e648e76))
* ttPvの伝播とRFPの安全弁を発動率不足で棄却する（ADR-0105） ([#108](https://github.com/kaz9120/himawari-rs/issues/108)) ([e9b28ef](https://github.com/kaz9120/himawari-rs/commit/e9b28ef1a5bdb1205ff79a649c6af109bcf95190))
* オートパイロットで進める前提をCLAUDE.mdに書く ([#105](https://github.com/kaz9120/himawari-rs/issues/105)) ([9ba0ebc](https://github.com/kaz9120/himawari-rs/commit/9ba0ebc88d51b7a40a0e0eda37e4cbaf301d0c11))
* ブランチ運用の規約を足す（ADR-0070） ([#68](https://github.com/kaz9120/himawari-rs/issues/68)) ([038e957](https://github.com/kaz9120/himawari-rs/commit/038e957f01998397cfa4f60cac63ea5857cc4b18))
* やねうら王との探索機能差分を棚卸ししてIDEASに反映する ([#8](https://github.com/kaz9120/himawari-rs/issues/8)) ([c4e6181](https://github.com/kaz9120/himawari-rs/commit/c4e618191e49980fc58988096c5b043871f3ec68))
* 入玉局面という見立てを取り下げる ([#123](https://github.com/kaz9120/himawari-rs/issues/123)) ([487bef8](https://github.com/kaz9120/himawari-rs/commit/487bef832af1fb18c8b6f9d9df2b600d557e04e8))
* 参照実装への追従を群単位で進める方針を決める（ADR-0109） ([8600555](https://github.com/kaz9120/himawari-rs/commit/8600555d86770062597fff582efeff3632a7f26f))
* 待機と判断を分ける運用上の注意を足す（ADR-0098） ([#80](https://github.com/kaz9120/himawari-rs/issues/80)) ([2ac400d](https://github.com/kaz9120/himawari-rs/commit/2ac400d857bac901daed7664e8151b585825c94c))
* 探索改善の選定基準を3軸で置く（ADR-0089） ([#53](https://github.com/kaz9120/himawari-rs/issues/53)) ([214cd77](https://github.com/kaz9120/himawari-rs/commit/214cd77a8515536c6d539cca460a50d995ac864c))
* 時間配分のmove horizon化を棄却した記録を残す（ADR-0102） ([#98](https://github.com/kaz9120/himawari-rs/issues/98)) ([aef504d](https://github.com/kaz9120/himawari-rs/commit/aef504db8a23a3519ebbf6a882f33b2e059207aa))
* 次の入口をNPSのプロファイルにする（ROADMAP） ([#86](https://github.com/kaz9120/himawari-rs/issues/86)) ([4efdf28](https://github.com/kaz9120/himawari-rs/commit/4efdf28866569c81505e9dded559f1eb8d42c733))
* 詰まされる側を打ち切らない根拠を実測で補う（ADR-0088） ([#64](https://github.com/kaz9120/himawari-rs/issues/64)) ([9b30cc9](https://github.com/kaz9120/himawari-rs/commit/9b30cc9d943a2c676ff4f7d9c63ea053dd653a08))
* 追従の目標を再現に置き、例外を1件へ絞る（ADR-0109） ([be92d0f](https://github.com/kaz9120/himawari-rs/commit/be92d0f099dbe5afd5a087c0ec11356bade31435))


### 内部

* NPS計測とプロファイルの手順をスクリプトへ固める（ADR-0081） ([#96](https://github.com/kaz9120/himawari-rs/issues/96)) ([ca0db43](https://github.com/kaz9120/himawari-rs/commit/ca0db43fcb243c02b2e4b791c1ff9908279a0608))
* release main ([#103](https://github.com/kaz9120/himawari-rs/issues/103)) ([165c521](https://github.com/kaz9120/himawari-rs/commit/165c52140a43290ee631c412c4fe6aa09f7ef4e4))
* release main ([#106](https://github.com/kaz9120/himawari-rs/issues/106)) ([c8e81a3](https://github.com/kaz9120/himawari-rs/commit/c8e81a32a32d91699218ac0f3a5e9cea439f8f1a))
* release main ([#109](https://github.com/kaz9120/himawari-rs/issues/109)) ([c8c7680](https://github.com/kaz9120/himawari-rs/commit/c8c7680e29622a8514147cb2cd29cfcca080be3d))
* release main ([#11](https://github.com/kaz9120/himawari-rs/issues/11)) ([f6fd0d6](https://github.com/kaz9120/himawari-rs/commit/f6fd0d63fdfd0ba91f38f664e3086fa6f10b6b21))
* release main ([#111](https://github.com/kaz9120/himawari-rs/issues/111)) ([9bf091f](https://github.com/kaz9120/himawari-rs/commit/9bf091ffc41ae202a78bd8887b9cabbcf4a096e0))
* release main ([#114](https://github.com/kaz9120/himawari-rs/issues/114)) ([04715db](https://github.com/kaz9120/himawari-rs/commit/04715dbcbe4a35cad1a95e80d4f8f9b88a2cc6e1))
* release main ([#118](https://github.com/kaz9120/himawari-rs/issues/118)) ([cf50978](https://github.com/kaz9120/himawari-rs/commit/cf50978e8cff453d48e3da5ca0f5ed5e647f7580))
* release main ([#120](https://github.com/kaz9120/himawari-rs/issues/120)) ([4e1654f](https://github.com/kaz9120/himawari-rs/commit/4e1654fc6c6193f0e42e85da87a150c588f2e4db))
* release main ([#124](https://github.com/kaz9120/himawari-rs/issues/124)) ([d6ee451](https://github.com/kaz9120/himawari-rs/commit/d6ee45112bbeadccc791a356ab62b34b8d98b85c))
* release main ([#125](https://github.com/kaz9120/himawari-rs/issues/125)) ([ac6f816](https://github.com/kaz9120/himawari-rs/commit/ac6f816bcee71c51a305c72260a0dedbbf8fbabb))
* release main ([#126](https://github.com/kaz9120/himawari-rs/issues/126)) ([cf587c5](https://github.com/kaz9120/himawari-rs/commit/cf587c545c5c626eb42c4949bcbea065e0f35a1d))
* release main ([#13](https://github.com/kaz9120/himawari-rs/issues/13)) ([20ed7ab](https://github.com/kaz9120/himawari-rs/commit/20ed7ab1d356ce21297ef7316ef9220a9c33a23e))
* release main ([#15](https://github.com/kaz9120/himawari-rs/issues/15)) ([156d02c](https://github.com/kaz9120/himawari-rs/commit/156d02c757a5e75ee233a92c0815575e8b8d54f6))
* release main ([#17](https://github.com/kaz9120/himawari-rs/issues/17)) ([19c2e18](https://github.com/kaz9120/himawari-rs/commit/19c2e18c1efe57077d360f75845297c2be8b906f))
* release main ([#19](https://github.com/kaz9120/himawari-rs/issues/19)) ([e1d4c81](https://github.com/kaz9120/himawari-rs/commit/e1d4c8189d65f0983c4273284d6ed055be2ee8af))
* release main ([#21](https://github.com/kaz9120/himawari-rs/issues/21)) ([078458b](https://github.com/kaz9120/himawari-rs/commit/078458bc6ec7cc86735c5020de85e4dac66e8c1c))
* release main ([#23](https://github.com/kaz9120/himawari-rs/issues/23)) ([20f8cd8](https://github.com/kaz9120/himawari-rs/commit/20f8cd805613071fc57306394f196f4e6395f0b8))
* release main ([#25](https://github.com/kaz9120/himawari-rs/issues/25)) ([11c15ed](https://github.com/kaz9120/himawari-rs/commit/11c15edb12da23708cdd1c8e2c7357422572eb49))
* release main ([#27](https://github.com/kaz9120/himawari-rs/issues/27)) ([652fc27](https://github.com/kaz9120/himawari-rs/commit/652fc2793a97ecd489224bba388a2c2c7473449b))
* release main ([#29](https://github.com/kaz9120/himawari-rs/issues/29)) ([baa8f0c](https://github.com/kaz9120/himawari-rs/commit/baa8f0c396d1ae90201650d32ba591a5c91fb955))
* release main ([#32](https://github.com/kaz9120/himawari-rs/issues/32)) ([dcb0d2d](https://github.com/kaz9120/himawari-rs/commit/dcb0d2d9f7a4bce1661064c1cd4dc24346428ef9))
* release main ([#33](https://github.com/kaz9120/himawari-rs/issues/33)) ([72dcd48](https://github.com/kaz9120/himawari-rs/commit/72dcd4834ad2b778fcf8b9d2e033cc8d775ef726))
* release main ([#35](https://github.com/kaz9120/himawari-rs/issues/35)) ([3a4834a](https://github.com/kaz9120/himawari-rs/commit/3a4834a7b4f86690ec000527a5826bf830919ce8))
* release main ([#37](https://github.com/kaz9120/himawari-rs/issues/37)) ([ac0aa90](https://github.com/kaz9120/himawari-rs/commit/ac0aa90ce0df65b47c250737348c77acb9efa884))
* release main ([#39](https://github.com/kaz9120/himawari-rs/issues/39)) ([9b7a12a](https://github.com/kaz9120/himawari-rs/commit/9b7a12ad255c7728d301db774d6dc89bd697f2e6))
* release main ([#41](https://github.com/kaz9120/himawari-rs/issues/41)) ([5f6b460](https://github.com/kaz9120/himawari-rs/commit/5f6b460e8351128858ffc09afe57c6e450f79408))
* release main ([#44](https://github.com/kaz9120/himawari-rs/issues/44)) ([9f103b3](https://github.com/kaz9120/himawari-rs/commit/9f103b30eb924a572b0fa5a2c244c5d63ea92780))
* release main ([#47](https://github.com/kaz9120/himawari-rs/issues/47)) ([32c2492](https://github.com/kaz9120/himawari-rs/commit/32c2492b0276800583062a95b029d75abf0f0cae))
* release main ([#5](https://github.com/kaz9120/himawari-rs/issues/5)) ([5ed2ac1](https://github.com/kaz9120/himawari-rs/commit/5ed2ac1ea8a1b5c03fce0a53299862c3a430bf14))
* release main ([#50](https://github.com/kaz9120/himawari-rs/issues/50)) ([679910c](https://github.com/kaz9120/himawari-rs/commit/679910cdb7d27eba0da627f6c3ff02e92ecabbee))
* release main ([#52](https://github.com/kaz9120/himawari-rs/issues/52)) ([1f0abd3](https://github.com/kaz9120/himawari-rs/commit/1f0abd3d871e792d8ea7250addf69f0e59209dcf))
* release main ([#55](https://github.com/kaz9120/himawari-rs/issues/55)) ([1a7d5b2](https://github.com/kaz9120/himawari-rs/commit/1a7d5b2cda033d8126bea05f33e996189e05a56d))
* release main ([#57](https://github.com/kaz9120/himawari-rs/issues/57)) ([84349b9](https://github.com/kaz9120/himawari-rs/commit/84349b9ce4453692458dc4276590f0d6a59e120d))
* release main ([#60](https://github.com/kaz9120/himawari-rs/issues/60)) ([3358269](https://github.com/kaz9120/himawari-rs/commit/3358269a2af98a808aaf10cc1a06d2c48e37f3b8))
* release main ([#61](https://github.com/kaz9120/himawari-rs/issues/61)) ([51d748a](https://github.com/kaz9120/himawari-rs/commit/51d748a2133de51164179b20c7b7cb22a236cccb))
* release main ([#63](https://github.com/kaz9120/himawari-rs/issues/63)) ([c7e110e](https://github.com/kaz9120/himawari-rs/commit/c7e110e906c63271d42db3c1798dbc5b12c4392e))
* release main ([#65](https://github.com/kaz9120/himawari-rs/issues/65)) ([c687136](https://github.com/kaz9120/himawari-rs/commit/c6871363b2a77a97027d5dc2219cf8cca1cdb766))
* release main ([#67](https://github.com/kaz9120/himawari-rs/issues/67)) ([7d3a2b5](https://github.com/kaz9120/himawari-rs/commit/7d3a2b53c2a3006d481e2e19ff6121044cb877db))
* release main ([#69](https://github.com/kaz9120/himawari-rs/issues/69)) ([08d4062](https://github.com/kaz9120/himawari-rs/commit/08d40626d3c966e243690cbb782ef0f65338d0f9))
* release main ([#7](https://github.com/kaz9120/himawari-rs/issues/7)) ([41c1cfa](https://github.com/kaz9120/himawari-rs/commit/41c1cfad6aa2856412ebb4806b077aef1d081632))
* release main ([#71](https://github.com/kaz9120/himawari-rs/issues/71)) ([192cb8a](https://github.com/kaz9120/himawari-rs/commit/192cb8a9e8d1a6ec87e7a58b330af3f4b3570999))
* release main ([#73](https://github.com/kaz9120/himawari-rs/issues/73)) ([5125002](https://github.com/kaz9120/himawari-rs/commit/5125002a0b21867c4f36ca3d682a53da241001d7))
* release main ([#75](https://github.com/kaz9120/himawari-rs/issues/75)) ([b59d82b](https://github.com/kaz9120/himawari-rs/commit/b59d82b830960d70f33944535b79d96138bac187))
* release main ([#77](https://github.com/kaz9120/himawari-rs/issues/77)) ([96034fa](https://github.com/kaz9120/himawari-rs/commit/96034facc68dfa6d446fb8a7d790400fd4dad4d8))
* release main ([#79](https://github.com/kaz9120/himawari-rs/issues/79)) ([f44f96c](https://github.com/kaz9120/himawari-rs/commit/f44f96c65b6389b0c89fb0a588f3ff19abef4e2a))
* release main ([#81](https://github.com/kaz9120/himawari-rs/issues/81)) ([df8d210](https://github.com/kaz9120/himawari-rs/commit/df8d2102f329f5500516cca9e66f2f4a4d586c60))
* release main ([#83](https://github.com/kaz9120/himawari-rs/issues/83)) ([fcd1d98](https://github.com/kaz9120/himawari-rs/commit/fcd1d985a2274ee724234bae8f07b3ae87226b00))
* release main ([#85](https://github.com/kaz9120/himawari-rs/issues/85)) ([d87fd85](https://github.com/kaz9120/himawari-rs/commit/d87fd852df3d74f3f80f54c103b158a373849183))
* release main ([#87](https://github.com/kaz9120/himawari-rs/issues/87)) ([8c25d01](https://github.com/kaz9120/himawari-rs/commit/8c25d0126aa2b9e7b17c932dd4d227807f3c9113))
* release main ([#9](https://github.com/kaz9120/himawari-rs/issues/9)) ([7043bfa](https://github.com/kaz9120/himawari-rs/commit/7043bfa9e67ab74f27fc19af7f6e16a1fe4548f8))
* release main ([#90](https://github.com/kaz9120/himawari-rs/issues/90)) ([3b9d0af](https://github.com/kaz9120/himawari-rs/commit/3b9d0af22ffafb1208559b8522d652efe6981659))
* release main ([#91](https://github.com/kaz9120/himawari-rs/issues/91)) ([cd530b2](https://github.com/kaz9120/himawari-rs/commit/cd530b2e9d7b97adac9fbf2f82a0e325f35ff953))
* release main ([#94](https://github.com/kaz9120/himawari-rs/issues/94)) ([0937a27](https://github.com/kaz9120/himawari-rs/commit/0937a277edacfffd7636ed9e0d4f73bd43186cf1))
* release main ([#95](https://github.com/kaz9120/himawari-rs/issues/95)) ([2bcf6ba](https://github.com/kaz9120/himawari-rs/commit/2bcf6ba3f30c9ecd251d32072f654cf84e51fd06))
* release main ([#97](https://github.com/kaz9120/himawari-rs/issues/97)) ([ea71da6](https://github.com/kaz9120/himawari-rs/commit/ea71da6a6b6eadde6607d453d7997c1587162291))
* release-pleaseのrelease-typeをsimpleに変える ([#4](https://github.com/kaz9120/himawari-rs/issues/4)) ([aaeb00c](https://github.com/kaz9120/himawari-rs/commit/aaeb00c67937de2491fd038e05fe263f2bde4c62))
* selfplayに両者ponderのモードを足す（--ponder-both） ([#99](https://github.com/kaz9120/himawari-rs/issues/99)) ([fd4bdb3](https://github.com/kaz9120/himawari-rs/commit/fd4bdb352ffc6e53d07a0ff30760120ba08baca9))
* SPRTの再開を自動化する（ADR-0087） ([#82](https://github.com/kaz9120/himawari-rs/issues/82)) ([9f09b1b](https://github.com/kaz9120/himawari-rs/commit/9f09b1b53464aa048334548bee333236b8d4dea0))
* SPRTの手順をスクリプトへ固める（ADR-0081） ([#78](https://github.com/kaz9120/himawari-rs/issues/78)) ([bfe2ea2](https://github.com/kaz9120/himawari-rs/commit/bfe2ea2032f20f30876b41a5e4e93f8cf2cb4c6a))
* エージェントが待機で止まらないようにする（ADR-0098） ([#76](https://github.com/kaz9120/himawari-rs/issues/76)) ([750e349](https://github.com/kaz9120/himawari-rs/commit/750e3495f82edaa0f48fd10aafc9eb06f9319b14))
* バージョン更新とリリースをrelease-pleaseへ委ねる（ADR-0071） ([#3](https://github.com/kaz9120/himawari-rs/issues/3)) ([218deda](https://github.com/kaz9120/himawari-rs/commit/218dedaaed2b02e0e559695d9a1b6a16305ac3cf))
* ライセンスをMITからGPLv3へ変更する（ADR-0108） ([8550b4d](https://github.com/kaz9120/himawari-rs/commit/8550b4d0601522394c9d63b7ed0c60cc024f9e8b))
* リリースPRでCargo.lockのバージョンを同期する（ADR-0071） ([#10](https://github.com/kaz9120/himawari-rs/issues/10)) ([f095e00](https://github.com/kaz9120/himawari-rs/commit/f095e009f696430fcf78ef0e582b3ccbf0b10ef8))
* 中断したSPRTを棋譜から再開できるようにする（ADR-0087） ([#49](https://github.com/kaz9120/himawari-rs/issues/49)) ([f86604c](https://github.com/kaz9120/himawari-rs/commit/f86604cdd5caff99116b6f52b12eb94a5fb3412f))
* 定跡の配布と生成条件の記録を整える（ADR-0082） ([#34](https://github.com/kaz9120/himawari-rs/issues/34)) ([4db7441](https://github.com/kaz9120/himawari-rs/commit/4db74414a0df9107e25b19c8a3620764f67c705b))
* 機能検証をスクリプト化し局面と深さを固定する ([#42](https://github.com/kaz9120/himawari-rs/issues/42)) ([29f7d19](https://github.com/kaz9120/himawari-rs/commit/29f7d19001379f19ae2bc690ab013a027284b8b3))
* 開発環境の再現手順をスクリプト化する（ADR-0080・0081） ([#30](https://github.com/kaz9120/himawari-rs/issues/30)) ([aa8716a](https://github.com/kaz9120/himawari-rs/commit/aa8716a1f95396ea6ff155e05d5755b78b16dfe9))

## [0.16.3](https://github.com/kaz9120/himawari-rs/compare/v0.16.2...v0.16.3) (2026-07-29)


### ドキュメント

* G0の範囲をStackの器に絞り、cutNodeの実引数表を足す（ADR-0109） ([2c40e76](https://github.com/kaz9120/himawari-rs/commit/2c40e7628c681e9511a8e8460625c4d7d42583eb))
* 参照実装への追従を群単位で進める方針を決める（ADR-0109） ([8600555](https://github.com/kaz9120/himawari-rs/commit/8600555d86770062597fff582efeff3632a7f26f))


### 内部

* ライセンスをMITからGPLv3へ変更する（ADR-0108） ([8550b4d](https://github.com/kaz9120/himawari-rs/commit/8550b4d0601522394c9d63b7ed0c60cc024f9e8b))

## [0.16.2](https://github.com/kaz9120/himawari-rs/compare/v0.16.1...v0.16.2) (2026-07-29)


### ドキュメント

* aspirationの窓外れが評価値とともに増えることを記録する ([#122](https://github.com/kaz9120/himawari-rs/issues/122)) ([6e7a134](https://github.com/kaz9120/himawari-rs/commit/6e7a13474dc258ec86f94fef9fdbc1b2b5bf37fc))

## [0.16.1](https://github.com/kaz9120/himawari-rs/compare/v0.16.0...v0.16.1) (2026-07-29)


### ドキュメント

* ponder有効時の思考時間1.25倍を棄却した記録を残す（ADR-0107） ([#121](https://github.com/kaz9120/himawari-rs/issues/121)) ([bd6179f](https://github.com/kaz9120/himawari-rs/commit/bd6179f20b6d909f04a473d4d0571e33b9b2ef09))

## [0.16.0](https://github.com/kaz9120/himawari-rs/compare/v0.15.9...v0.16.0) (2026-07-29)


### 棋力向上

* USIの入出力をファイルへ写すDebugLogFileを足す ([#117](https://github.com/kaz9120/himawari-rs/issues/117)) ([0eea97c](https://github.com/kaz9120/himawari-rs/commit/0eea97c629975f2389dbca4b24d6047996ec3e0f))


### ドキュメント

* floodgateの対局から入玉局面の評価という論点を立てる ([#119](https://github.com/kaz9120/himawari-rs/issues/119)) ([18eac68](https://github.com/kaz9120/himawari-rs/commit/18eac68a3a64448e57e492e9948235af3d63b372))

## [0.15.9](https://github.com/kaz9120/himawari-rs/compare/v0.15.8...v0.15.9) (2026-07-29)


### その他の変更

* 打ち切り時に未確定のaspiration窓外れを最後に残さない ([#116](https://github.com/kaz9120/himawari-rs/issues/116)) ([94bd980](https://github.com/kaz9120/himawari-rs/commit/94bd98078eaeed4dc8bde9e0d81fa7ecfeceea44))

## [0.15.8](https://github.com/kaz9120/himawari-rs/compare/v0.15.7...v0.15.8) (2026-07-29)


### ドキュメント

* 2026-07-29の時間管理キャンペーンをROADMAPに残す ([#112](https://github.com/kaz9120/himawari-rs/issues/112)) ([638cc4f](https://github.com/kaz9120/himawari-rs/commit/638cc4fa97e24ab3c27cc0398cd82c053b4bb1c7))
* ponderhitでの探索継続を棄却した記録を残す（ADR-0106） ([#113](https://github.com/kaz9120/himawari-rs/issues/113)) ([07dd208](https://github.com/kaz9120/himawari-rs/commit/07dd20811a6aba80fd3c80f1db59e4d3e5cef4fe))

## [0.15.7](https://github.com/kaz9120/himawari-rs/compare/v0.15.6...v0.15.7) (2026-07-29)


### ドキュメント

* ttPvの伝播とRFPの安全弁を発動率不足で棄却する（ADR-0105） ([#108](https://github.com/kaz9120/himawari-rs/issues/108)) ([e9b28ef](https://github.com/kaz9120/himawari-rs/commit/e9b28ef1a5bdb1205ff79a649c6af109bcf95190))

## [0.15.6](https://github.com/kaz9120/himawari-rs/compare/v0.15.5...v0.15.6) (2026-07-29)


### ドキュメント

* ponderの時間会計を棄却した記録を残す（ADR-0104） ([#107](https://github.com/kaz9120/himawari-rs/issues/107)) ([262fc06](https://github.com/kaz9120/himawari-rs/commit/262fc0679d488e26c97fa2d0decbf877f6f5b802))

## [0.15.5](https://github.com/kaz9120/himawari-rs/compare/v0.15.4...v0.15.5) (2026-07-29)


### ドキュメント

* rootの1位2位差を判定材料に足す案を実装前に棄却する（ADR-0103） ([#102](https://github.com/kaz9120/himawari-rs/issues/102)) ([217fe96](https://github.com/kaz9120/himawari-rs/commit/217fe963f4c39e7957ad047ec67cda55beeba151))
* オートパイロットで進める前提をCLAUDE.mdに書く ([#105](https://github.com/kaz9120/himawari-rs/issues/105)) ([9ba0ebc](https://github.com/kaz9120/himawari-rs/commit/9ba0ebc88d51b7a40a0e0eda37e4cbaf301d0c11))

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
