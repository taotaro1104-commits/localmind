//! 実モデル（e5）で、質問と各チャンクの cosine を測る。資料ごとの足切りの値を決めるための測定用。
//! 使い方: cargo run --release -p lm-workspace --features onnx-measure --example cutoff -- <モデルの展開先ディレクトリ>

use lm_search::vector::cosine;
use lm_search::{Embedder, OnnxEmbedder};
use lm_workspace::{split, Format};

const DOCS: [(&str, &str); 3] = [
    ("契約書", "# 業務委託契約書\n\n## 第12条（解除）\n\n契約書の解除は第12条に定める。解除するには30日前までに書面で通知する。\n\n## 第15条（秘密保持）\n\n受託者は、業務で知り得た委託者の情報を、契約の終了後も第三者に開示してはならない。\n"),
    ("就業規則", "# 就業規則\n\n## 休暇\n\n年次有給休暇は入社6か月後に10日付与され、取得は1時間単位でも申請できる。\n\n## 経費\n\n出張の経費精算は帰着後5営業日以内に申請する。\n取引先との会食は1人あたり1万円を上限とし、事前に部門長の承認を得る。\n\n## 在宅勤務\n\n在宅勤務は前日の17時までに上長へ申請する。緊急の場合は当日の朝9時までに連絡する。\n"),
    ("議事録", "# 議事録の取り扱い規程\n\n## 保存と削除\n\n議事録の保存期間は30日とし、期限を過ぎた録音は自動的に削除される。\n\n## 録音の同意\n\n録音を始める前に、参加者全員の同意を確認する。同意のない録音は保存しない。\n"),
];

/// (質問, 答えがある見出し（複数なら `|` 区切り）。無ければ空)
const QUERIES: [(&str, &str); 12] = [
    ("契約をやめたいときはどうすればいいですか", "第12条（解除）"),
    ("録音データはいつ消えますか", "保存と削除"),
    ("出張の経費はいつまでに精算しますか", "経費"),
    ("在宅勤務はいつまでに申請しますか", "在宅勤務"),
    ("有給休暇は何日もらえますか", "休暇"),
    ("秘密保持はいつまで続きますか", "第15条（秘密保持）"),
    ("録音の前に何を確認しますか", "録音の同意"),
    (
        "契約の解除と秘密保持について教えてください",
        "第12条（解除）|第15条（秘密保持）",
    ),
    ("第12条には何が書いてありますか", "第12条（解除）"),
    ("経費精算と会食の上限を教えてください", "経費"),
    ("宇宙旅行の費用はいくらですか", ""),
    ("今日の天気はどうですか", ""),
];

fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("モデルの展開先を指定してください");
    let model = OnnxEmbedder::from_bundled_dir(&dir).expect("モデルの読み込み");
    let mut chunks = Vec::new();
    for (title, text) in DOCS {
        for c in split(text, Format::Markdown).unwrap() {
            let v = model.embed_document(&c.indexed_text()).unwrap();
            chunks.push((title, c.heading_path.last().cloned().unwrap_or_default(), v));
        }
    }
    for (q, answer) in QUERIES {
        let qv = model.embed_query(q).unwrap();
        let mut scored: Vec<(f32, &str, &str)> = chunks
            .iter()
            .map(|(t, h, v)| (cosine(&qv, v), *t, h.as_str()))
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!(
            "\n質問: {q}  （答え: {}）",
            if answer.is_empty() { "なし" } else { answer }
        );
        let top = scored[0].0;
        for (cos, t, h) in scored.iter().take(5) {
            let mark = if !answer.is_empty() && *h == answer {
                "◎"
            } else {
                " "
            };
            println!("  {mark} cos={cos:.3}  top差={:.3}  {t} / {h}", top - cos);
        }
    }
}
