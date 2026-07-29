# Changelog

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
