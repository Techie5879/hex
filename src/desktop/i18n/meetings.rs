//! Meeting interface strings added once [`super::TRANSLATIONS`] reached its
//! file's size limit, in the same six columns: Polish, Chinese (Simplified),
//! Japanese, German, Spanish, Ukrainian. Traditional Chinese is converted
//! from the Simplified column as for every other string.

/// Every six-column string: [`super::TRANSLATIONS`], then these.
pub(super) fn every() -> impl Iterator<Item = &'static (&'static str, [&'static str; 6])> {
    super::TRANSLATIONS.iter().chain(TRANSLATIONS)
}

#[rustfmt::skip]
const TRANSLATIONS: &[(&str, [&str; 6])] = &[
    ("Shared room", ["Wspólna sala", "共享房间", "共有ルーム", "Gemeinsamer Raum", "Sala compartida", "Спільна кімната"]),
    ("Only this recording. Use Room mic and Call audio for people sharing this room.", ["Tylko to nagranie. Użyj etykiet Mikrofon sali i Dźwięk rozmowy dla osób we wspólnej sali.", "仅限本次录制。使用“房间麦克风”和“通话音频”标记同处一室的人。", "この録音のみ。同じ部屋にいる人には「室内マイク」と「通話音声」を使います。", "Nur diese Aufnahme. Verwende Raummikrofon und Anruf-Audio für Personen im selben Raum.", "Solo esta grabación. Usa Micrófono de la sala y Audio de la llamada para quienes comparten esta sala.", "Лише цей запис. Використовуйте «Мікрофон кімнати» та «Звук дзвінка» для людей у спільній кімнаті."]),
    ("Saving meeting mode…", ["Zapisywanie trybu spotkania…", "正在保存会议模式…", "会議モードを保存中…", "Meeting-Modus wird gespeichert…", "Guardando el modo de reunión…", "Збереження режиму зустрічі…"]),
    ("Room mic", ["Mikrofon sali", "房间麦克风", "室内マイク", "Raummikrofon", "Micrófono de la sala", "Мікрофон кімнати"]),
    ("Call audio", ["Dźwięk rozmowy", "通话音频", "通話音声", "Anruf-Audio", "Audio de la llamada", "Звук дзвінка"]),
    ("Shared room: Room mic is everyone near this computer and Call audio is the call. Hex doesn't tell the people apart.", ["Wspólna sala: Mikrofon sali to wszyscy w pobliżu tego komputera, a Dźwięk rozmowy to rozmowa. Hex nie rozróżnia tych osób.", "共享房间：“房间麦克风”是这台电脑附近的所有人，“通话音频”是通话本身。Hex 不区分其中的人。", "共有ルーム：「室内マイク」はこのコンピュータの近くにいる全員、「通話音声」は通話です。Hex は人を区別しません。", "Gemeinsamer Raum: Raummikrofon sind alle in der Nähe dieses Computers, Anruf-Audio ist der Anruf. Hex unterscheidet die Personen nicht.", "Sala compartida: Micrófono de la sala es todo el que está cerca de este ordenador y Audio de la llamada es la llamada. Hex no distingue a las personas.", "Спільна кімната: «Мікрофон кімнати» — усі поруч із цим комп’ютером, «Звук дзвінка» — сам дзвінок. Hex не розрізняє людей."]),
];
