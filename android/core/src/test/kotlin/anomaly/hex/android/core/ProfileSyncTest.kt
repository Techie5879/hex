package anomaly.hex.android.core

import anomaly.hex.android.core.profile.PortableProfile
import anomaly.hex.android.core.profile.ProfileEditor
import anomaly.hex.android.core.profile.StampClock
import anomaly.hex.android.core.sync.ProfileStore
import anomaly.hex.android.core.sync.ProfileSync
import anomaly.hex.android.core.sync.ProfileTransport
import anomaly.hex.android.core.sync.RemoteDocument
import anomaly.hex.android.core.sync.SyncConflictException
import anomaly.hex.android.core.sync.SyncGaveUpException
import java.io.IOException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

private class MemoryStore(var localDoc: PortableProfile = PortableProfile.empty()) : ProfileStore {
    var ancestorDoc = PortableProfile.empty()
    override fun updateLocal(change: (PortableProfile) -> PortableProfile): PortableProfile {
        localDoc = change(localDoc)
        return localDoc
    }
    override fun ancestor() = ancestorDoc
    override fun saveAncestor(profile: PortableProfile) { ancestorDoc = profile }
}

/** A server with real compare-and-swap, plus hooks to lose a race or go offline. */
private class FakeServer : ProfileTransport {
    var text: String? = null
    var version = 0
    var beforeStore: (() -> Unit)? = null
    var offline = false
    var stores = 0

    override fun fetch(): RemoteDocument {
        if (offline) throw IOException("offline")
        return RemoteDocument(text, text?.let { "v$version" })
    }

    override fun store(text: String, ifMatch: String?): String {
        if (offline) throw IOException("offline")
        beforeStore.also { beforeStore = null }?.invoke()
        if (ifMatch != this.text?.let { "v$version" }) throw SyncConflictException()
        this.text = text
        version += 1
        stores += 1
        return "v$version"
    }
}

class ProfileSyncTest {
    private fun editor(device: String, at: Long) = ProfileEditor(StampClock(device) { at })

    @Test
    fun twoDevicesConvergeIncludingOfflineEditsAndDeletes() {
        val server = FakeServer()
        val phone = MemoryStore(editor("phone", 1000).putTerm(PortableProfile.empty(), "Nimbus"))
        val mac = MemoryStore(editor("mac", 1100).putTerm(PortableProfile.empty(), "Quartz"))
        ProfileSync(server, phone).sync()
        ProfileSync(server, mac).sync()
        ProfileSync(server, phone).sync()
        assertEquals(phone.localDoc.encode(), mac.localDoc.encode())
        assertEquals(listOf("Nimbus", "Quartz"), phone.localDoc.liveTerms.map { it.term })

        // Phone is offline, deletes a term and adds another. Mac keeps editing.
        server.offline = true
        assertThrows(IOException::class.java) { ProfileSync(server, phone).sync() }
        phone.localDoc = editor("phone", 2000).removeTerm(phone.localDoc, "Quartz")
        phone.localDoc = editor("phone", 2001).putTerm(phone.localDoc, "Offline")
        assertThrows(IOException::class.java) { ProfileSync(server, phone).sync() }
        assertEquals(listOf("Nimbus", "Offline"), phone.localDoc.liveTerms.map { it.term })

        server.offline = false
        mac.localDoc = editor("mac", 2100).putTerm(mac.localDoc, "Macside")
        ProfileSync(server, mac).sync()
        ProfileSync(server, phone).sync()
        ProfileSync(server, mac).sync()
        assertEquals(phone.localDoc.encode(), mac.localDoc.encode())
        assertEquals(listOf("Macside", "Nimbus", "Offline"), mac.localDoc.liveTerms.map { it.term })
        assertTrue(mac.localDoc.vocabulary.getValue("quartz").deleted)
    }

    @Test
    fun aLostRaceRefetchesAndKeepsBothEdits() {
        val server = FakeServer()
        val other = MemoryStore(editor("mac", 1000).putTerm(PortableProfile.empty(), "Racing"))
        val phone = MemoryStore(editor("phone", 1000).putTerm(PortableProfile.empty(), "Nimbus"))
        server.beforeStore = { ProfileSync(FakeServerProxy(server), other).sync() }
        val outcome = ProfileSync(server, phone).sync()
        assertEquals(2, outcome.attempts)
        assertEquals(listOf("Nimbus", "Racing"), PortableProfile.decode(server.text!!).liveTerms.map { it.term })
    }

    @Test
    fun anEditDuringAnyStepOfSyncSurvivesAndGoesUpNextTime() {
        val server = FakeServer()
        val phone = MemoryStore(editor("phone", 1000).putTerm(PortableProfile.empty(), "Nimbus"))
        // A person adds a word while the upload is on its way.
        server.beforeStore = { phone.localDoc = editor("phone", 2000).putTerm(phone.localDoc, "Duringupload") }
        ProfileSync(server, phone).sync()
        assertEquals(listOf("Duringupload", "Nimbus"), phone.localDoc.liveTerms.map { it.term })
        assertEquals(listOf("Nimbus"), PortableProfile.decode(server.text!!).liveTerms.map { it.term })
        ProfileSync(server, phone).sync()
        assertEquals(listOf("Duringupload", "Nimbus"), PortableProfile.decode(server.text!!).liveTerms.map { it.term })

        // And while the document is being fetched.
        val fetching = object : ProfileTransport by server {
            override fun fetch(): RemoteDocument {
                phone.localDoc = editor("phone", 3000).putTerm(phone.localDoc, "Duringfetch")
                return server.fetch()
            }
        }
        ProfileSync(fetching, phone).sync()
        assertTrue("Duringfetch" in PortableProfile.decode(server.text!!).liveTerms.map { it.term })
    }

    @Test
    fun anIdleSyncUploadsNothing() {
        val server = FakeServer()
        val phone = MemoryStore(editor("phone", 1000).putTerm(PortableProfile.empty(), "Nimbus"))
        assertTrue(ProfileSync(server, phone).sync().uploaded)
        assertFalse(ProfileSync(server, phone).sync().uploaded)
        assertEquals(1, server.stores)
    }

    @Test
    fun aRejectedRemoteLeavesLocalEditsAlone() {
        val server = FakeServer().apply { text = """{"schema":"hex.profile","version":99}""" }
        val phone = MemoryStore(editor("phone", 1000).putTerm(PortableProfile.empty(), "Nimbus"))
        val before = phone.localDoc
        assertThrows(Exception::class.java) { ProfileSync(server, phone).sync() }
        assertEquals(before, phone.localDoc)
        assertEquals(PortableProfile.empty(), phone.ancestorDoc)
    }

    @Test
    fun aServerThatKeepsChangingEndsInARetryNotALoop() {
        val phone = MemoryStore(editor("phone", 1000).putTerm(PortableProfile.empty(), "Nimbus"))
        val always = object : ProfileTransport {
            override fun fetch() = RemoteDocument(null, null)
            override fun store(text: String, ifMatch: String?): String = throw SyncConflictException()
        }
        assertThrows(SyncGaveUpException::class.java) { ProfileSync(always, phone, maxAttempts = 3).sync() }
        assertEquals(1, phone.localDoc.liveTerms.size)
    }
}

private class FakeServerProxy(private val server: FakeServer) : ProfileTransport by server
