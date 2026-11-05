package com.localmind.sample

import android.app.job.JobParameters
import android.app.job.JobService
import kotlin.concurrent.thread

/** 1 日ごとに動き、保存期間を過ぎた録音と、古い一時ファイルを消す。 */
class PurgeJob : JobService() {
    override fun onStartJob(params: JobParameters): Boolean {
        thread(name = "lm-purge-job") {
            try {
                // 失敗しても次の周期でやり直す（再試行はしない）。
                Purger.run(applicationContext)
            } finally {
                jobFinished(params, false) // 例外でも、ジョブを終わらせる
            }
        }
        return true
    }

    override fun onStopJob(params: JobParameters): Boolean = false
}
