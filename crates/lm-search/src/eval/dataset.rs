//! 評価用の小さな日本語コーパスと質問セット（G-6）。
//!
//! 質問は3種類: 自然文／識別子（型番・人名・条文番号）／表記ゆれ。
//! `expected = None` は資料に答えが無い質問（「見つかりません」が正解）。

pub struct EvalCase {
    pub query: &'static str,
    pub expected: Option<&'static str>,
}

pub const CORPUS: &[(&str, &str)] = &[
    ("contract-renewal", "業務委託契約の更新日は来年三月三十一日です。更新の通知は期限の三十日前までに書面で行います。"),
    ("contract-secrecy", "第12条 秘密保持義務 受託者は業務上知り得た情報を第三者に開示してはならない。"),
    ("contract-penalty", "第15条 損害賠償 本契約に違反した当事者は、相手方に生じた損害を賠償する責任を負う。"),
    ("manual-ab1234", "型番 AB-1234 の取扱説明書。電源ボタンを三秒間長押しすると初期化されます。"),
    ("manual-xr900", "XR-900 ワイヤレスヘッドホンの充電時間は約二時間、連続再生は三十時間です。"),
    ("manual-router", "無線ルーターの設置場所は壁から離し、床から一メートル以上の高さに置いてください。"),
    ("minutes-0412", "四月十二日の定例会議。決定事項: 新製品の発売日を九月一日とする。担当は田中さん。"),
    ("minutes-0510", "五月十日の企画会議。議題は展示会の出展準備。山田さんが見積もりを来週までに提出する。"),
    ("minutes-todo", "ToDo: 佐藤さんは契約書の最終版を金曜日までに法務部へ送付する。"),
    ("recipe-curry", "カレーの作り方。玉ねぎを飴色になるまで炒め、鶏肉とじゃがいもを加えて煮込みます。"),
    ("recipe-ramen", "自家製ラーメンのスープは鶏ガラと昆布を二時間煮出し、醤油ダレで味を調えます。"),
    ("travel-kyoto", "京都旅行の計画。一日目は清水寺と祇園、二日目は嵐山で竹林の道を歩く予定です。"),
    ("travel-hotel", "宿泊先は新大阪駅近くのホテル。チェックインは十五時、チェックアウトは十一時です。"),
    ("study-tcp", "TCP は信頼性のある通信を提供するプロトコルで、三ウェイハンドシェイクで接続を確立します。"),
    ("study-sql", "SQL のインデックスは検索を高速化するが、更新時にはインデックスの再構築コストがかかる。"),
    ("health-sleep", "睡眠時間は成人で七時間前後が目安とされ、就寝前のスマートフォン利用は控えるのがよい。"),
    ("finance-tax", "確定申告の期限は三月十五日です。医療費控除を受けるには領収書の保管が必要です。"),
    ("server-migration", "サーバーの移行作業は夜間に実施し、データベースのバックアップを事前に取得する。"),
    ("car-maintenance", "自動車のエンジンオイルは五千キロまたは半年ごとに交換するのが推奨されています。"),
    ("garden", "トマトの苗は日当たりの良い場所に植え、支柱を立てて風で倒れないようにします。"),
];

pub const CASES: &[EvalCase] = &[
    // --- 自然文 ---
    EvalCase {
        query: "契約の更新日はいつですか",
        expected: Some("contract-renewal"),
    },
    EvalCase {
        query: "更新の通知はいつまでに行う？",
        expected: Some("contract-renewal"),
    },
    EvalCase {
        query: "秘密保持の義務について教えて",
        expected: Some("contract-secrecy"),
    },
    EvalCase {
        query: "契約違反したら損害賠償はどうなる",
        expected: Some("contract-penalty"),
    },
    EvalCase {
        query: "ヘッドホンの充電時間は何時間",
        expected: Some("manual-xr900"),
    },
    EvalCase {
        query: "ルーターはどこに置けばいい",
        expected: Some("manual-router"),
    },
    EvalCase {
        query: "新製品の発売日はいつに決まった",
        expected: Some("minutes-0412"),
    },
    EvalCase {
        query: "展示会の見積もりは誰が出す",
        expected: Some("minutes-0510"),
    },
    EvalCase {
        query: "契約書の最終版は誰がいつまでに送る",
        expected: Some("minutes-todo"),
    },
    EvalCase {
        query: "カレーに入れる具材は",
        expected: Some("recipe-curry"),
    },
    EvalCase {
        query: "京都旅行の二日目の予定",
        expected: Some("travel-kyoto"),
    },
    EvalCase {
        query: "ホテルのチェックアウト時間",
        expected: Some("travel-hotel"),
    },
    EvalCase {
        query: "TCP はどうやって接続を確立する",
        expected: Some("study-tcp"),
    },
    EvalCase {
        query: "インデックスのデメリットは",
        expected: Some("study-sql"),
    },
    EvalCase {
        query: "確定申告はいつまで",
        expected: Some("finance-tax"),
    },
    EvalCase {
        query: "エンジンオイルの交換時期",
        expected: Some("car-maintenance"),
    },
    EvalCase {
        query: "トマトの育て方",
        expected: Some("garden"),
    },
    // --- 識別子（型番・人名・条文番号） ---
    EvalCase {
        query: "AB-1234",
        expected: Some("manual-ab1234"),
    },
    EvalCase {
        query: "XR-900",
        expected: Some("manual-xr900"),
    },
    EvalCase {
        query: "第12条",
        expected: Some("contract-secrecy"),
    },
    EvalCase {
        query: "第15条",
        expected: Some("contract-penalty"),
    },
    EvalCase {
        query: "第十二条",
        expected: Some("contract-secrecy"),
    },
    EvalCase {
        query: "第十五条の損害賠償",
        expected: Some("contract-penalty"),
    },
    EvalCase {
        query: "山田さん",
        expected: Some("minutes-0510"),
    },
    EvalCase {
        query: "佐藤さん",
        expected: Some("minutes-todo"),
    },
    // --- 表記ゆれ ---
    EvalCase {
        query: "ＡＢ－１２３４ の初期化方法",
        expected: Some("manual-ab1234"),
    },
    EvalCase {
        query: "ｻｰﾊﾞ の移行はいつやる",
        expected: Some("server-migration"),
    },
    EvalCase {
        query: "サーバの移行手順",
        expected: Some("server-migration"),
    },
    EvalCase {
        query: "ラｰメンのスープの作り方",
        expected: Some("recipe-ramen"),
    },
    // --- 資料に無い質問 ---
    EvalCase {
        query: "量子コンピュータの冷却方式",
        expected: None,
    },
    EvalCase {
        query: "明日の東京の天気予報",
        expected: None,
    },
    EvalCase {
        query: "ビットコインの現在の価格",
        expected: None,
    },
    EvalCase {
        query: "サッカーワールドカップの優勝国",
        expected: None,
    },
    EvalCase {
        query: "ピザ生地の発酵時間",
        expected: None,
    },
    EvalCase {
        query: "ハワイ旅行のおすすめホテル",
        expected: None,
    },
    EvalCase {
        query: "マンションの管理費の相場",
        expected: None,
    },
    EvalCase {
        query: "英語の不規則動詞の一覧",
        expected: None,
    },
    EvalCase {
        query: "zzzzqqqq",
        expected: None,
    },
    EvalCase {
        query: "ZZ-9999",
        expected: None,
    },
    EvalCase {
        query: "第99条",
        expected: None,
    },
    EvalCase {
        query: "第十三条",
        expected: None,
    },
    EvalCase {
        query: "第九十九条の規定",
        expected: None,
    },
    EvalCase {
        query: "鈴木さんの連絡先",
        expected: None,
    },
];
