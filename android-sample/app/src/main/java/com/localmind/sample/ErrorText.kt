package com.localmind.sample

/**
 * Rust 側（`lm-workspace`・`lm-llm`・`lm-ingest` など）のエラーの文言は日本語。英語の画面では、断片ごとに英語へ置き換える。
 * 表に無い文言が残ったら null を返し、呼び出し側が汎用の文言（英語）にする（日本語を英語の画面に出さない）。
 * 日本語の画面では、そのまま返す。
 */
internal object ErrorText {
    private class Rule(val ja: Regex, val en: String)

    private fun rule(ja: String, en: String) = Rule(Regex(ja), en)

    // 長いもの・具体的なものを先に置く（短い断片が、長い文の一部を先に置き換えないように）
    private val RULES = listOf(
        // ワークスペース
        rule("""文書が大きすぎます（(\d+) バイト。上限は (\d+) バイト）""", "The document is too large (\$1 bytes; the limit is \$2 bytes)"),
        rule("""文書が大きすぎます（(\d+) バイトまで。分割して登録してください）""", "The document is too large (up to \$1 bytes; please split it up)"),
        rule("""文書の区切りが多すぎます（上限は (\d+) 個）。分けて登録してください""", "The document has too many sections (the limit is \$1). Please split it up"),
        rule("""登録できる資料がいっぱいです（(\d+) 件、または合計 (\d+) バイトまで）。不要な資料を削除してください""",
            "The document store is full (up to \$1 documents or \$2 bytes in total). Please delete documents you do not need"),
        rule("""文書に使えない文字が含まれています: """, "The document contains characters that cannot be used: "),
        rule("""NUL 文字""", "NUL character"),
        rule("""文書の id が不正です（小文字の英数字・ハイフン・アンダースコアで 1〜64 文字。OS の予約名は使えません）""",
            "The document id is invalid (1 to 64 lowercase letters, digits, hyphens or underscores; OS reserved names are not allowed)"),
        rule("""題名が不正です: """, "The title is invalid: "),
        rule("""文書が見つかりません""", "The document was not found"),
        rule("""保存した文書を読めません（壊れているか、形式が違います）""", "Cannot read the saved document (it is damaged or in a different format)"),
        rule("""文書の保存に失敗しました: """, "Could not save the document: "),
        rule("""資料は保存しましたが、検索の準備に失敗しました（再構築で直せます）: """,
            "The document was saved, but preparing it for search failed (a rebuild can fix this): "),
        rule("""グループ名に改行・制御文字・見えない文字は使えません""", "A group name cannot contain line breaks, control characters or invisible characters"),
        rule("""グループ名が長すぎます""", "The group name is too long"),
        rule("""(?<=: )空です$""", "it is empty"),
        rule("""(?<=: )長すぎます$""", "it is too long"),
        rule("""改行・制御文字・見えない文字は使えません""", "line breaks, control characters and invisible characters cannot be used"),
        rule("""権限がありません""", "Permission denied"),
        rule("""保存領域が足りません""", "Not enough storage space"),
        rule("""保存先を作れません""", "Cannot create the folder to save to"),
        rule("""書き込めません""", "Cannot write"),
        rule("""一覧を読めません""", "Cannot read the list"),
        rule("""削除できません""", "Cannot delete"),
        // 録音・文字起こし
        rule("""不正な設定: """, "Invalid setting: "),
        rule("""不正な録音 id: """, "Invalid recording id: "),
        rule("""入出力エラー: """, "I/O error: "),
        rule("""データが壊れています: """, "The data is damaged: "),
        rule("""議事録の抽出に失敗しました: """, "Extracting the minutes failed: "),
        rule("""録音の同意が確認されていません: """, "Consent to record has not been confirmed: "),
        rule("""文字起こしに失敗しました: """, "Transcription failed: "),
        rule("""文字起こしエンジンの初期化に失敗しました""", "Could not initialize the transcription engine"),
        rule("""文字起こしの途中で失敗しました""", "Transcription failed partway"),
        rule("""この録音の議事録はまだありません""", "There are no minutes for this recording yet"),
        rule("""この録音は終了しています""", "This recording has already ended"),
        rule("""その録音がありません""", "That recording does not exist"),
        // 議事録・質問応答（LLM）
        rule("""発話の数が多すぎます""", "There are too many utterances"),
        rule("""文字起こしが大きすぎます""", "The transcript is too large"),
        rule("""発話の時刻が不正です（時刻の順で、重ならないこと）""", "The utterance times are invalid (they must be in order and must not overlap)"),
        rule("""文字起こしが長すぎます。短い区間に分けて抽出してください""", "The transcript is too long. Please split it into shorter sections and extract them separately"),
        rule("""文字起こしが長すぎて、モデルが扱えません""", "The transcript is too long for the model"),
        rule("""議事録の抽出の途中で失敗しました""", "Extracting the minutes failed partway"),
        rule("""質問が長すぎます。短くして、もう一度お試しください""", "The question is too long. Please shorten it and try again"),
        rule("""質問が長すぎます（(\d+) 文字まで）""", "The question is too long (up to \$1 characters)"),
        rule("""質問を入力してください""", "Please type a question"),
        rule("""回答の作成の途中で失敗しました""", "Writing the answer failed partway"),
        rule("""回答を作れませんでした""", "Could not make an answer"),
        rule("""検索の要否を読めませんでした""", "Could not read whether a search is needed"),
        rule("""LLM の出力を読めません: """, "Cannot read the LLM output: "),
        rule("""JSON ではありません（出力が途中で切れた可能性があります）""", "it is not JSON (the output may have been cut off)"),
        rule("""出力が大きすぎます""", "the output is too large"),
        rule("""(\w+) がありません""", "\$1 is missing"),
        rule("""中止されました""", "Stopped"),
        // モデル（LLM エンジン・ファイルの検証）
        rule("""LLM エンジンの初期化に失敗しました""", "Could not initialize the LLM engine"),
        rule("""LLM エンジンを初期化できません""", "Could not initialize the LLM engine"),
        rule("""スレッド数は 1〜(\d+) でなければなりません""", "The number of threads must be 1 to \$1"),
        rule("""文脈長が範囲外です（512〜32768）""", "The context length is out of range (512 to 32768)"),
        rule("""制限時間が範囲外です（10〜3600 秒）""", "The time limit is out of range (10 to 3600 seconds)"),
        rule("""モデルファイルが検証のあとに変更されました。置き直してください""", "The model file changed after it was checked. Please put it in place again"),
        rule("""モデルを読み込めません（形式が違うか、メモリが足りません）""", "Cannot load the model (wrong format, or not enough memory)"),
        rule("""時間がかかりすぎたので止めました。短い文字起こしでお試しください""", "Stopped because it took too long. Please try a shorter transcript"),
        rule("""推論の準備に失敗しました（メモリが足りません）""", "Could not prepare for inference (not enough memory)"),
        rule("""出力の形の指定が不正です""", "The output format specification is invalid"),
        rule("""推論に失敗しました""", "Inference failed"),
        rule("""出力を文字として読めません""", "Cannot read the output as text"),
        rule("""モデルファイルを使えません: """, "Cannot use the model file: "),
        rule("""期待するハッシュは小文字の 16 進 64 文字で指定してください""", "The expected hash must be 64 lowercase hexadecimal characters"),
        rule("""内容が固定したハッシュと一致しません（破損か差し替え）""", "the contents do not match the pinned hash (damaged or replaced)"),
        rule("""検証している間に変更されました""", "it changed while being checked"),
        rule("""大きさが想定の範囲外です""", "the size is out of the expected range"),
        rule("""通常のファイルではありません""", "it is not a regular file"),
        rule("""調べられません""", "cannot inspect"),
        rule("""開けません""", "cannot open"),
        rule("""(?<=: )読めません$""", "cannot read"),
        rule("""読み込めません""", "Cannot read"),
        // パスの名前
        rule("""は `\.\.` を含まない絶対パスで指定してください""", " must be an absolute path without '..'"),
        rule("""モデルのパス""", "The model path"),
        rule("""録音の置き場所""", "The recordings folder"),
        rule("""録音の保存先""", "The recordings folder"),
        rule("""索引のパス""", "The index path"),
        rule("""文書の保存先""", "The documents folder"),
        rule("""モデルのディレクトリ""", "The model folder"),
        // 種類の見出し（エラーの型の表示用）
        rule("""ワークスペースのエラー: """, "Workspace error: "),
        rule("""録音のエラー: """, "Recording error: "),
        rule("""検索エラー: """, "Search error: "),
        rule("""経路エラー: """, "Routing error: "),
        rule("""埋め込みモデルのエラー: """, "Embedding model error: "),
        rule("""モデルファイルの整合性エラー: """, "Model file integrity error: "),
        rule("""入力のエラー: """, "Input error: "),
        rule("""LLM のエラー: """, "LLM error: "),
    )

    private val JAPANESE = Regex("[぀-ヿ一-鿿＀-￯]")

    /** 画面に出す文言。英語の画面で、訳せない日本語が残ったら null。 */
    fun localize(msg: String?): String? {
        if (msg == null) return null
        if (I18n.lang == Lang.JA) return msg
        var out: String = msg
        for (r in RULES) out = r.ja.replace(out, r.en)
        return out.takeUnless { JAPANESE.containsMatchIn(it) }
    }
}
