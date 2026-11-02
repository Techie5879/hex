package anomaly.hex.android.core.sync

import anomaly.hex.android.core.profile.PortableProfile
import anomaly.hex.android.core.profile.ProfileMerge

/** One remote profile document plus its version tag. Null text means none exists. */
data class RemoteDocument(val text: String?, val etag: String?)

class SyncConflictException : Exception("The remote profile changed during upload")

/** The whole remote contract: read a document, write it only if it hasn't changed. */
interface ProfileTransport {
    fun fetch(): RemoteDocument
    /** Throws [SyncConflictException] on a version mismatch. Returns the new tag, if any. */
    fun store(text: String, ifMatch: String?): String?
}

/** What sync reads and durably writes on this device. Local data outlives any transport failure. */
interface ProfileStore {
    /** Applies [change] to the current local copy as one atomic step, so no edit can land between. */
    fun updateLocal(change: (PortableProfile) -> PortableProfile): PortableProfile
    fun ancestor(): PortableProfile
    fun saveAncestor(profile: PortableProfile)
}

data class SyncOutcome(val profile: PortableProfile, val uploaded: Boolean, val attempts: Int)

class SyncGaveUpException : Exception("The remote profile kept changing; will try again later")

class ProfileSync(
    private val transport: ProfileTransport,
    private val store: ProfileStore,
    private val maxAttempts: Int = 5,
) {
    /**
     * Fetch, merge, keep the merge locally, then upload with compare-and-swap.
     * A lost race refetches. Any failure leaves local edits saved and retried later;
     * the ancestor moves only once the server holds the same document. An edit made
     * after the merge stays local, differs from the ancestor, and goes up next time.
     */
    fun sync(): SyncOutcome {
        repeat(maxAttempts) { index ->
            val remoteDoc = transport.fetch()
            val remote = remoteDoc.text?.let(PortableProfile::decode) ?: PortableProfile.empty()
            val ancestor = store.ancestor()
            val merged = store.updateLocal { local ->
                ProfileMerge.merge(local, remote, ancestor).also(PortableProfile::validate)
            }
            val encoded = merged.encode()
            val upload = remoteDoc.text == null || encoded != remote.encode()
            if (upload) {
                try {
                    transport.store(encoded, remoteDoc.etag)
                } catch (_: SyncConflictException) {
                    return@repeat
                }
            }
            store.saveAncestor(merged)
            return SyncOutcome(merged, upload, index + 1)
        }
        throw SyncGaveUpException()
    }
}
