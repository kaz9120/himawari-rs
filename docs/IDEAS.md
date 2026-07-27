# アイデア帳

ADRより手前の受け皿。1案1行で気軽に追記する。着手を決めたら
ADRを起草してこの帳から消す（採番と経緯はADR側に書く）。
棄却したら理由を1行残して ~~打ち消し~~ にする。
検証手段の略記: SPRT=対局ゲート、loss=検証損失、NPS=速度計測。

他の案を前提とする案は、メモ欄に依存先を書く。表の並び順は優先度を
表さない。ADR-0072はhistory pruningを単独で着手できると見なして失敗した。
着手前に依存を確かめる（[ADR-0074](adr/0074-feature-verification.md)）。

## 探索: 枝刈り・延長・オーダリング

2026-07-27にやねうら王と機能差分を棚卸しした。比較対象は
yaneurao/YaneuraOu master の 0899d1dc（Stockfish 17〜18系の移植）で、
`source/engine/yaneuraou-engine/yaneuraou-search.cpp` ほかを参照した。
以下4節の未着手案の多くはその差分に由来し、メモ中の式はやねうら王側の
実装値である。差分はムーブループ内の枝刈り・LMRの項・historyの種類・
時間配分式の4領域に集中する。

過去にSPRTで棄却した案のうち、P3期（駒割評価）の測定は条件が違う。
singular extensionがP3の-16.0からP8で+12.6に反転した先例があり、
NNUE後の再測定に意味がある。

| 案 | 狙い・メモ | 検証 |
|---|---|---|
| ~~singular extension再挑戦~~ | ~~ADR-0050で導入。+12.6 Elo（H1採択、P3の-16.0から反転）~~ | ~~完了~~ |
| ~~ProbCut~~ | ~~ADR-0051で導入。+44.2 Elo（H1採択）~~ | ~~完了~~ |
| ~~razoring~~ | ~~ADR-0057で導入。+184.8 Elo（H1採択）~~ | ~~完了~~ |
| ~~qsearchのTT保存拡充~~ | ~~ADR-0054で導入（probe+store+TT手、同一キーガード込み）。+113.6 Elo（H1採択）~~ | ~~完了~~ |
| lmrDepth基準の枝刈り | 現状は生depthで判断する。lmrDepth = newDepth - r/1024 とhistory補正を導入する。下の枝刈り4件の土台 | SPRT |
| quiet SEE pruning再挑戦 | ムーブループ内で see_ge(m, -25*lmrDepth^2)。P3で+0.9（2266局）だが駒割評価時代の測定 | SPRT |
| capture futility | staticEval + 218 + 223*lmrDepth + 駒価値 + captHist項。capture historyが前提 | SPRT |
| capture SEE pruning | see_ge(m, -max(167*depth + captHist*34/1024, 0))。capture historyが前提 | SPRT |
| ttPvフラグの活用 | TTに保存済みだが探索で読んでいない。LMR・RFP・singularの条件へ供給する。ほぼゼロコスト | SPRT |
| ~~razoringの深さ制限撤廃~~ | ~~ADR-0075で棄却。マージンの向きを取り違えていた。razoringはマージンが大きいほど刈りにくく、本エンジンの300はやねうら王より緩い。揃えるとノードが5.4倍に増える~~ | ~~棄却~~ |
| RFPの項追加とマージン緩和 | 4種の枝刈りで唯一、本エンジンが刈りにくい側にある（ADR-0075）。現状 120*depth・depth<=6。やねうら王は 76*depth・depth<15 にimprovingとopponentWorseningの項が付いてさらに減る。返り値の(2*beta+eval)/3化も含む | SPRT |
| ProbCutの条件緩和 | depth>=5を3へ、マージン200固定を224-61*improvingへ、SEE閾値0をprobCutBeta-staticEvalへ | SPRT |
| TTベースの簡易ProbCut | ムーブループ直前。ttBoundがLOWERかつttDepth>=depth-4かつttValue>=beta+416で即return | SPRT |
| ~~qsearchのfutility~~ | ~~ADR-0077で導入。+57.3 Elo（H1採択、1242局）。bestValueの引き上げはMultiPVの整合のため入れていない~~ | ~~完了~~ |
| singularマージンの将棋適合 | 現状 ttValue - 2*depth はStockfishの値のまま。やねうら王は係数を1/55へ下げ「1割がsingularになるよう調整」と明記 | SPRT |
| singularの多段化 | double/triple extension、multi-cut、negative extension。ADR-0050は単独延長のみ | SPRT |
| IIRの条件精緻化 | 現状 depth>=4 かつTT手なし。allNode除外と親のreduction量に連動した深さ±1を足す | SPRT |
| mate1plyの探索組み込み再挑戦 | P3で+1.1（968局、駒割時代）で見送り。やねうら王はTTミス時だけ呼びコストを抑える | SPRT |
| ~~LMRの固定小数化~~ | ~~ADR-0076で導入。機能検証で3局面とも完全一致（等価変更）を示しSPRTを省いた。1024倍のスケールはやねうら王と2.4%差で一致し、項の重みを換算せずに使える~~ | ~~完了~~ |
| LMRのcutNode項 | やねうら王の最大の項（+3611 + 985*!ttMove）。searchにcutNode引数を足して伝播させる配管が要る。リダクションを強める唯一の大項で、他の弱める項の前提になる | SPRT |
| LMRのttPv項（再挑戦） | ADR-0076で単独導入したところ-43.8のH0。弱める方向にしか働かず、cutNode項がないため釣り合いが取れなかった。cutNode導入後に再挑戦する | SPRT |
| LMRのその他の項 | rootDelta・moveCount・correction値・ttCapture・cutoffCnt・ttMove一致・statScore・allNode。1項=1SPRT。固定小数化（ADR-0076）済みで粒度は確保した | SPRT |
| LMR再探索の深さ調整 | doDeeperSearch（value > bestValue+48）とdoShallowerSearch（value < bestValue+9）でnewDepthを±1する | SPRT |
| NMPのR拡大と検証探索 | ADR-0052は+7.8 [-0.7,+16.2]（6264局、判定未了で保留、adr-0052-wipブランチ）。R=3+d/4は小さい。R=7+d/3・cutNode限定・nmpMinPlyで再挑戦 | SPRT保留 |
| aspiration窓の再調整 | delta 20固定を 5+|meanSquaredScore|/9000 へ、中心を前深さのスコアからaverageScoreへ、拡大をdelta/3へ。fail-high時にrootDepthを削る | SPRT |
| 王手・取り返し延長の精査 | やねうら王は王手延長も取り返し延長も持たない（Stockfishが削除、将棋では王手が続きやりすぎになる）。現状のgives_check延長が過剰でないかを測る | SPRT |
| df-pn詰み探索 | 長手数詰み。終盤力・宣言勝ち周りの取りこぼし対策 | SPRT |
| SEE駒価値のNNUE時代適合 | 枝刈り閾値系の駒価値を再調整 | SPRT |

## 探索: history

| 案 | 狙い・メモ | 検証 |
|---|---|---|
| ~~correction history~~ | ~~ADR-0046で導入。+44.6 Elo（H1採択）~~ | ~~完了~~ |
| ~~continuation history~~ | ~~ADR-0047で導入。+20.7 Elo（H1採択）~~ | ~~完了~~ |
| ~~capture history~~ | ~~ADR-0048で不採択（872局で-2.4、効果なし打ち切り）。MVV-LVAとのスケール再設計なら再挑戦可~~ | ~~不採択~~ |
| capture history再挑戦 | 上限10692・初期値-678のgravity方式へ作り替える。capture系の枝刈り2件の前提になる | SPRT |
| correction historyの多系統化 | 現状はpawn keyのみで+44.6 Elo。minor piece・nonPawn（先手/後手）・continuation（2手前/4手前）を足して重み合成する。core側にキーの追加が要る | SPRT |
| continuation historyの段数拡張 | 現状2段（1手前・2手前）。6手前まで重み付きにし、[王手中][捕獲]の4系統へ分ける。ADR-0073の後に行う。現状のテーブルは0.24%しか埋まっておらず、bonusが小さいまま系統を増やすとさらに疎になって逆効果 | SPRT |
| main historyの次元拡張 | 現状[駒32][移動先81]でfromも打ちの区別も持たない。[手番][move.raw()]へ。衝突の実測が先 | SPRT |
| statScoreの導入 | 捕獲は駒価値+captHist、静かな手は2*mainHist+cont[0]+cont[1]。LMRとbonus式の両方へ供給する | SPRT |
| pawn history | 歩の配置を条件にした指し手履歴。history pruningの入力にもなる | SPRT |
| lowPly history | ply<5専用の履歴。毎イテレーション98で初期化する | SPRT |
| historyのdivisor調整 | ADR-0073でbonus/malusを上げた結果、main historyの非ゼロ平均が6590へ上がり最大値が平衡点16384に飽和した。やねうら王の上限は7183。divisorを下げて解像度を戻せるか | SPRT |
| history bonusのttMove一致項と後方減衰 | ADR-0073は基本式のみを扱う。bonusへ 353*(bestMove==ttMove) を足し、malusを後方の手ほど ×977/1024 で減衰させる。外れた手の順序を保持する必要がある | SPRT |
| TTカット時のhistory更新 | ttValue>=betaでカットするとき、静かなTT手のhistoryと直前手のcontinuation historyを更新する | SPRT |

## 探索: 時間管理

floodgateの負け11局は終盤の時間枯渇だった（RESULTS.md）。この節は
その対処に直結する。

| 案 | 狙い・メモ | 検証 |
|---|---|---|
| 時間配分式のmove_horizon化 | ADR-0021の初期式のまま。MTG = min(max_moves_to_draw - ply + 2, 160±ply補正)/2。切れ負けと秒読みで分岐させ、終盤に厚く配る | SPRT |
| 秒単位切り上げと使い切り | stopを立てる代わりにsearch_endを設定し、秒単位まで使い切る。maximumがavail*3に張り付く問題も同時に見直す | SPRT |
| bestMoveChangesのスレッド集約 | 全スレッドの最善手変更回数を集約し 1.04+1.8956*changes/threads を掛ける。現状はメインのstable_itersのみ | SPRT |
| 安定度のロジスティック化 | 現状は 1.5-0.15*n の線形。0.723+0.79/(1.104+exp(-0.5189*(depth-center)))、center=lastBestMoveDepth+11.57 | SPRT |
| 最小思考時間とponder延長 | MinimumThinkingTime（やねうら王の既定は2000ms）と、ponder有効時にoptimumを1.25倍する扱い | SPRT |
| 時間管理: fail-low延長 | ADR-0059は評価下落で伸ばす。root fail-low時の明示的な延長は未着手 | SPRT |

## 探索: 並列・置換表

| 案 | 狙い・メモ | 検証 |
|---|---|---|
| ~~TTのbucket化・prefetch~~ | ~~bucket化はADR-0022で実装済み。prefetchはADR-0056で不採択（144局で-33.9、Apple Siliconで効果なし）~~ | ~~不採択~~ |
| ~~eval hash~~ | ~~ADR-0049で導入。+54.1 Elo、NPS +10.4%（H1採択）~~ | ~~完了~~ |
| best thread voting | 現状はメインワーカーの結果のみ採用する。(score-minScore+14)*completedDepth で投票させる。Threads>=4で効く | SPRT |
| Lazy SMPのdelta多様化 | aspiration初期deltaに threadIdx%8 を足す。現状は全ワーカーが同一探索で、多様化源がTT到着順しかない | SPRT |
| historyのスレッド共有 | pawn historyとcorrection historyをatomicで共有する（やねうら王はNUMA単位）。現状は全てスレッドローカル | SPRT |
| TTクラスタの手番分割 | インデックスの最下位ビットを手番で置換し、手番違いの衝突を消す | SPRT |
| draw valueのオプション化 | contempt相当。DrawValueBlack/White（やねうら王の既定は歩の-2%）。強い相手に引き分けを許容する | SPRT |
| 投了値（ResignValue） | 評価値が閾値を下回ったら投了する。現状は詰みでのみ投了する | 動作 |

## ネットワーク構造

| 案 | 狙い・メモ | 検証 |
|---|---|---|
| ~~factorizer（学習時のみK/P分解）~~ | ~~ADR-0066で導入。+28.1 Elo（H1採択、2772局）~~ | ~~完了~~ |
| output bucket（駒数で層分岐） | 局面フェーズ別の専用ヘッド。SF系で効果大 | SPRT |
| PSQT直結パス（material head） | FTから評価へのskip。序盤の材料感を安定化 | loss+SPRT |
| ~~FT次元拡大 256→512~~ | ~~ADR-0067で不採択（968局で-72.8）。train lossは0.0034下がったがNPS 0.65倍の損が上回る。長い持ち時間での再評価は可能~~ | ~~不採択~~ |
| ~~利き塔の差分計算化~~ | ~~ADR-0045で利き塔自体を除去（棋力寄与ゼロ）~~ | ~~棄却~~ |
| ~~利き塔の特徴変種~~ | ~~利き塔除去に伴い棄却~~ | ~~棄却~~ |
| 玉ライン特徴（8方向最近接駒） | ADR-0044で検証。valid lossで利き塔の87%を低コストで実現。差分計算可能 | loss+SPRT |
| 手番特徴の明示化 | HalfKPは手番を視点でしか持たない。tempo項の学習 | loss |
| 多ヘッド出力（WDL・進行度・安定度） | 時間管理・枝刈り強度・contemptへの供給源（ADRバックログ済み） | SPRT |
| 量子化の再検討（FT i8等） | メモリ帯域半減でNPS向上。ADR-0067でFT512がNPS律速と分かったため優先度が上がった。512の容量を活かす本命 | NPS+SPRT |

## 学習

| 案 | 狙い・メモ | 検証 |
|---|---|---|
| ~~train/validの対局単位分割~~ | ~~P5で実施済み（validは別thread由来）~~ | ~~完了~~ |
| ~~過学習対策一式~~ | ~~P6でearly stopping実装済み~~ | ~~完了~~ |
| ~~データ量スケーリング実証~~ | ~~P6で15M/86M/180Mの3点で実証済み~~ | ~~完了~~ |
| ~~valid lossカーブの可視化~~ | ~~P6でTSV+TensorBoard実装済み~~ | ~~完了~~ |
| 教師局面のqsearch静止化 | データセットREADMEも指摘。静止局面で学習し評価の整合を取る | loss+SPRT |
| ~~lrスケジュール~~ | ~~P6でwarmup+cosine decay実装済み~~ | ~~完了~~ |
| λのply依存化 | 序盤はresult重視、終盤はscore重視の混合比 | loss+SPRT |
| 詰みスコアの扱い | ±30000近傍のclamp/除外/専用ターゲット。現状8.5%を素通し | loss+SPRT |
| ミラーデータ拡張 | 左右反転で実質2倍。盤面対称性の担保にも | loss |
| EMA重み平均（SWA） | 終盤の重み振動を平均化。ほぼ無料で数Elo | SPRT |
| 継続学習の世代運用 | 前世代ネットからfinetune。lr小さめで積む | SPRT |
| 教師との指し手一致率の計測 | PSVのmoveフィールド活用。lossと別の健全性指標 | 指標 |
| 複数データセット混合 | hao系+水匠系等。分布の偏り緩和（ライセンス確認） | SPRT |
| 序中終盤のサンプリング重み | gamePly別の採択率調整 | loss+SPRT |
| 公開ネットウォームスタート対照 | ADR-0039の対照実験。互換ローダで初期化 | SPRT |
| ハイパラのSPRT運用 | lr・λ・epochsを1変更=1SPRTで積む | SPRT |

## 教師データ・基盤

| 案 | 狙い・メモ | 検証 |
|---|---|---|
| ~~学習スループット改善~~ | ~~ADR-0064/0065で41,000→449,000 samples/s。目標20万/秒を達成~~ | ~~完了~~ |
| ~~ストリーミングチャンクシャッフル~~ | ~~ADR-0065で実装。psv shuffleを2パスのバケット法にし、79.7GBを3分でシャッフル~~ | ~~完了~~ |
| 学習チェックポイント | f32状態の保存・再開。長時間学習の中断耐性 | 動作 |
| gensfen自前実装 | 24点法裁定・開始局面ランダム化・乱択（バックログ済み） | 動作 |
| RL世代ループ | 自分の深い探索を教師に世代を積む（バックログ済み） | SPRT |
| 大規模対局基盤 | SPRTの並列度向上、複数マシン分散 | 運用 |
| floodgate参戦 | 実戦レーティングの定点観測。対人間系の弱点発見 | 運用 |
| NPS回帰のCI監視 | ベンチ局面のNPSをCIで記録し、退行を検知 | 運用 |
| ~~定跡の方針決定~~ | ~~ADR-0060で方針決定（持つ・db形式互換・データは配布物に含めない）。実装は別ADR~~ | ~~完了~~ |
| ~~定跡ローダの実装~~ | ~~ADR-0063で起草（USI層で引く、db形式互換、book genで511局面を生成）~~ | ~~完了~~ |
| 定跡の掘り下げ・ランダム化 | ADR-0063は決定的に評価値最大を選ぶ。同じ相手に同じ負け方を繰り返すなら重み付け選択を検討 | 運用 |
| 定跡ヒット時のponder | 現状は定跡ヒット時にponder手を返さずponderが走らない。ponderhit時も定跡を引かず再探索する。定跡が続く間もTTを埋める価値があるが、engine側に「bestmoveを出さない停止」APIが要り影響範囲が大きい（2026-07-25、将来対応と判断） | 運用 |
