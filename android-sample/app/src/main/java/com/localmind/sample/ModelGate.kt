package com.localmind.sample

import java.util.concurrent.atomic.AtomicReference

/**
 * LLM のモデルを使う処理（議事録の作成・質問への回答）は、同時に 1 つだけ。数 GB のモデルを 2 つ開かないための、
 * 唯一の入口。確認してから実行するのではなく、取れたときだけ実行する（確認と実行のあいだに隙間を作らない）。
 */
internal object ModelGate {
    const val MINUTES = "minutes"
    const val ASK = "ask"

    private val owner = AtomicReference<String?>(null)

    /** 取れたら true。取れた側は、終わったら必ず [leave] する。 */
    fun tryEnter(who: String): Boolean = owner.compareAndSet(null, who)

    fun leave(who: String) {
        owner.compareAndSet(who, null)
    }

    fun isHeldBy(who: String): Boolean = owner.get() == who

    /** どちらかが使っているか。 */
    fun isBusy(): Boolean = owner.get() != null
}
