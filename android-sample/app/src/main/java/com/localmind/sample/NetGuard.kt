package com.localmind.sample

import java.io.IOException
import java.net.InetAddress
import java.net.Socket
import javax.net.ssl.SSLSocketFactory

/**
 * 利用者が入れた URL への接続で、自端末・社内ネットワーク・予約済みのアドレスへ向かわせないための検査。
 * 「公開されたアドレスだけ許す」（拒否の一覧ではなく、許可の側で判定する）。
 */
internal object NetGuard {
    /** 公開のインターネット上のアドレスか。 */
    fun isPublic(a: InetAddress): Boolean {
        if (a.isAnyLocalAddress || a.isLoopbackAddress || a.isLinkLocalAddress || a.isSiteLocalAddress || a.isMulticastAddress) return false
        val b = a.address.map { it.toInt() and 0xff }
        return when (b.size) {
            4 -> isPublicV4(b)
            16 -> isPublicV6(b)
            else -> false
        }
    }

    private fun isPublicV4(b: List<Int>): Boolean = when {
        b[0] == 0 || b[0] >= 240 -> false // 0.0.0.0/8、予約済み・ブロードキャスト（240.0.0.0/4）
        b[0] == 10 || b[0] == 127 -> false // 10.0.0.0/8、127.0.0.0/8
        b[0] == 172 && b[1] in 16..31 -> false // 172.16.0.0/12
        b[0] == 192 && b[1] == 168 -> false // 192.168.0.0/16
        b[0] == 169 && b[1] == 254 -> false // リンクローカル 169.254.0.0/16
        b[0] in 224..239 -> false // マルチキャスト
        b[0] == 100 && b[1] in 64..127 -> false // 共有アドレス空間（キャリア NAT）100.64.0.0/10
        b[0] == 192 && b[1] == 0 && b[2] == 0 -> false // 192.0.0.0/24
        b[0] == 192 && b[1] == 0 && b[2] == 2 -> false // ドキュメント用 192.0.2.0/24
        b[0] == 198 && b[1] in 18..19 -> false // ベンチマーク用 198.18.0.0/15
        b[0] == 198 && b[1] == 51 && b[2] == 100 -> false // ドキュメント用
        b[0] == 203 && b[1] == 0 && b[2] == 113 -> false // ドキュメント用
        else -> true
    }

    private fun isPublicV6(b: List<Int>): Boolean {
        val embeddedV4 = { at: Int -> isPublicV4(b.subList(at, at + 4)) }
        return when {
            (b[0] and 0xfe) == 0xfc -> false // ユニークローカル fc00::/7
            b[0] == 0xfe && (b[1] and 0xc0) == 0x80 -> false // リンクローカル
            b[0] == 0x20 && b[1] == 0x01 && b[2] == 0x0d && b[3] == 0xb8 -> false // ドキュメント用 2001:db8::/32
            b[0] == 0x20 && b[1] == 0x02 -> embeddedV4(2) // 6to4: 埋め込まれた IPv4 を調べる
            b.subList(0, 12) == listOf(0, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0) -> embeddedV4(12) // NAT64
            b.subList(0, 10).all { it == 0 } && b[10] == 0xff && b[11] == 0xff -> embeddedV4(12) // IPv4 射影
            b.subList(0, 15).all { it == 0 } -> false // :: と ::1
            else -> true
        }
    }

    /** ホスト名を解決した結果が、すべて公開アドレスか。解決できない・1 つでも非公開なら false。 */
    fun resolvesOnlyToPublic(host: String): Boolean = try {
        InetAddress.getAllByName(host).all { isPublic(it) }
    } catch (_: Exception) {
        false
    }

    /**
     * 接続の「直前」に、実際に接続した相手のアドレスを検査する TLS の工場。名前の解決と接続のあいだに宛先を
     * 差し替えられても（DNS リバインディング）、非公開のアドレスへは TLS を始めさせない。
     * 標準の接続は、先に素の接続を張ってから、この工場に TLS の層を重ねさせる。
     */
    class Guarded(private val delegate: SSLSocketFactory) : SSLSocketFactory() {
        private fun check(s: Socket?) {
            val peer = s?.inetAddress
            if (peer == null || !isPublic(peer)) {
                runCatching { s?.close() }
                throw IOException("このアドレスには接続できません")
            }
        }

        override fun createSocket(s: Socket?, host: String?, port: Int, autoClose: Boolean): Socket {
            check(s)
            return delegate.createSocket(s, host, port, autoClose)
        }

        override fun getDefaultCipherSuites(): Array<String> = delegate.defaultCipherSuites
        override fun getSupportedCipherSuites(): Array<String> = delegate.supportedCipherSuites

        // 名前から自分で接続する形は使わない（標準の接続は使わない）。使われたら、検査できないので断る。
        override fun createSocket(host: String?, port: Int): Socket = throw IOException("未対応の接続方法です")
        override fun createSocket(host: String?, port: Int, localHost: InetAddress?, localPort: Int): Socket =
            throw IOException("未対応の接続方法です")
        override fun createSocket(host: InetAddress?, port: Int): Socket = throw IOException("未対応の接続方法です")
        override fun createSocket(address: InetAddress?, port: Int, localAddress: InetAddress?, localPort: Int): Socket =
            throw IOException("未対応の接続方法です")
    }
}
