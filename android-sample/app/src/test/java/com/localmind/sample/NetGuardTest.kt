package com.localmind.sample

import java.net.InetAddress
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class NetGuardTest : JapaneseUi() {
    private fun ip(s: String) = InetAddress.getByName(s)

    @Test
    fun publicAddressesAreAllowed() {
        for (a in listOf("8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111", "2a00:1450:4001:81b::200e")) {
            assertTrue(a, NetGuard.isPublic(ip(a)))
        }
    }

    @Test
    fun localPrivateAndReservedRangesAreRefused() {
        val bad = listOf(
            "0.0.0.0", "0.1.2.3", "10.0.0.1", "127.0.0.1", "169.254.1.1", "172.16.0.1", "172.31.255.255", "192.168.1.1",
            "100.64.0.1", "100.127.255.254", "192.0.0.1", "192.0.2.1", "198.18.0.1", "198.19.255.255", "198.51.100.1",
            "203.0.113.1", "224.0.0.1", "239.255.255.255", "240.0.0.1", "255.255.255.255",
            "::", "::1", "fe80::1", "fc00::1", "fd12:3456::1", "ff02::1", "2001:db8::1",
            "::ffff:127.0.0.1", "::ffff:10.0.0.1", "64:ff9b::7f00:1", "64:ff9b::a00:1", "2002:7f00:1::1", "2002:a00:1::1",
        )
        for (a in bad) assertFalse(a, NetGuard.isPublic(ip(a)))
    }

    @Test
    fun embeddedPublicIpv4InTranslationPrefixesIsAllowed() {
        assertTrue(NetGuard.isPublic(ip("::ffff:8.8.8.8")))
        assertTrue(NetGuard.isPublic(ip("64:ff9b::808:808")))
    }

    @Test
    fun namesThatDoNotResolveAreRefused() {
        assertFalse(NetGuard.resolvesOnlyToPublic("no-such-host.invalid"))
        assertFalse(NetGuard.resolvesOnlyToPublic("localhost"))
    }
}
