//! wav ファイルから、アプリが保存するのと同じ形式の録音（メタ + 30 秒ごとのチャンク）を作る。
//! 実機・エミュレータで、実際の発話の文字起こしを試すための開発用の道具（マイクが無音のエミュレータなど）。
//!
//! 使い方: cargo run -p lm-ingest --example import_wav -- <wav（16bit・モノラル）> <出力先のルート> <録音id>
//! 出力先のルートの下に `<録音id>/` ができる。アプリの `files/recordings/` へ置けば、一覧に出る。

use lm_ingest::{ChunkWriter, FsChunkStore, RecordingMeta};

/// 16bit・モノラルの wav から、サンプルレートと PCM を取り出す（`data` チャンクを探す最小限の読み取り）。
fn read_wav(path: &str) -> (u32, Vec<i16>) {
    let b = std::fs::read(path).expect("wav を読めません");
    assert!(b.len() > 44 && &b[0..4] == b"RIFF", "wav ではありません");
    let rate = u32::from_le_bytes(b[24..28].try_into().unwrap());
    let channels = u16::from_le_bytes(b[22..24].try_into().unwrap());
    assert_eq!(channels, 1, "モノラルの wav だけ読めます");
    let format = u16::from_le_bytes(b[20..22].try_into().unwrap());
    let bits = u16::from_le_bytes(b[34..36].try_into().unwrap());
    assert!(
        format == 1 && bits == 16,
        "16bit の PCM の wav だけ読めます"
    );
    assert!(
        (8_000..=96_000).contains(&rate),
        "サンプルレートが範囲外です"
    );
    let mut at = 12;
    while at + 8 <= b.len() {
        let size = u32::from_le_bytes(b[at + 4..at + 8].try_into().unwrap()) as usize;
        if &b[at..at + 4] == b"data" {
            let data = &b[at + 8..(at + 8 + size).min(b.len())];
            let pcm = data
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| i16::from_le_bytes(*s))
                .collect();
            return (rate, pcm);
        }
        at += 8 + size;
    }
    panic!("data チャンクがありません");
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (wav, root, id) = match (args.next(), args.next(), args.next()) {
        (Some(w), Some(r), Some(i)) => (w, r, i),
        _ => {
            eprintln!("使い方: import_wav <wav> <出力先のルート> <録音id>");
            std::process::exit(2);
        }
    };
    eprintln!(
        "注意: この道具は、録音に「同意を得た」印を付けて作ります。自分で用意した音声（同意を得たもの）だけを使ってください。"
    );
    let (rate, pcm) = read_wav(&wav);
    let store = FsChunkStore::new(&root);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("時計が不正です")
        .as_millis() as u64;
    store
        .begin_recording(
            &id,
            &RecordingMeta {
                consent_confirmed: true, // 開発用の道具: 自分で用意した音声だけを使うこと
                sample_rate: rate,
                started_at_ms: now_ms,
            },
        )
        .expect("録音を始められません");
    let mut writer = ChunkWriter::standard(rate).expect("チャンクの設定");
    let mut chunks = writer.push(&pcm).expect("分割できません");
    chunks.extend(writer.finish());
    for chunk in &chunks {
        store.save_chunk(&id, chunk).expect("保存できません");
    }
    println!(
        "{id}: {} チャンク（{} 秒）を {root} に作りました",
        chunks.len(),
        pcm.len() as u32 / rate
    );
}
