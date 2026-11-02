package anomaly.hex.android.core.profile

import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonObject

/**
 * Three-way merge against the last profile this device agreed with the server.
 * Each field: an unchanged side takes the other. When both sides changed:
 * dictionary terms keep the newest stamp and union their spellings; deletes
 * win or lose by stamp alone; settings keep the newest and record the loser
 * as an open conflict, so a concurrent prompt edit is never silently dropped.
 * The result is the same whichever device runs it.
 */
object ProfileMerge {
    fun merge(local: PortableProfile, remote: PortableProfile, ancestor: PortableProfile): PortableProfile {
        val conflicts = (local.conflicts.keys + remote.conflicts.keys).associateWith { id ->
            mergeConflict(local.conflicts[id], remote.conflicts[id])
        }.toMutableMap()

        val vocabulary = (local.vocabulary.keys + remote.vocabulary.keys).associateWith { key ->
            mergeTerm(local.vocabulary[key], remote.vocabulary[key], ancestor.vocabulary[key])
        }

        val settings = (local.settings.keys + remote.settings.keys).associateWith { key ->
            threeWay(local.settings[key], remote.settings[key], ancestor.settings[key]) { l, r ->
                when {
                    l.value == r.value -> if (l.stamp >= r.stamp) l else r
                    else -> {
                        val (winner, loser) = if (l.stamp >= r.stamp) l to r else r to l
                        val conflict = Conflict(key, loser.value, loser.stamp, winner.stamp)
                        if (conflict.id !in conflicts) conflicts[conflict.id] = conflict
                        winner
                    }
                }
            }
        }
        return PortableProfile(
            vocabulary = vocabulary,
            settings = settings,
            conflicts = conflicts,
            extra = JsonObject(remote.extra + local.extra),
        )
    }

    private fun mergeTerm(local: VocabEntry?, remote: VocabEntry?, ancestor: VocabEntry?): VocabEntry =
        threeWay(local, remote, ancestor) { l, r ->
            val (winner, loser) = if (l.stamp >= r.stamp) l to r else r to l
            when {
                winner.deleted -> winner
                loser.deleted -> winner
                else -> winner.copy(soundsLike = (winner.soundsLike + loser.soundsLike).distinct().take(MAX_SOUNDS_LIKE))
            }
        }

    private fun mergeConflict(local: Conflict?, remote: Conflict?): Conflict {
        val a = local ?: remote!!
        val b = remote ?: local!!
        val resolved = listOfNotNull(a.resolvedStamp, b.resolvedStamp).maxOrNull()
        return a.copy(value = if (resolved != null) JsonNull else a.value, resolvedStamp = resolved)
    }

    private inline fun <T : Any> threeWay(local: T?, remote: T?, ancestor: T?, bothChanged: (T, T) -> T): T = when {
        local == null -> remote!!
        remote == null -> local
        local == remote -> local
        local == ancestor -> remote
        remote == ancestor -> local
        else -> bothChanged(local, remote)
    }
}
