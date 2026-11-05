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
    ("Shared room meetings", ["Spotkania we wspólnej sali", "共享房间会议", "共有ルームの会議", "Meetings im gemeinsamen Raum", "Reuniones en sala compartida", "Зустрічі в спільній кімнаті"]),
    ("For calls where others in your room speak into this computer's microphone. Lines say Room mic and Call audio instead of You and Them, and room speech the call repeats word for word shows once. Starts with the next recording.", ["Do rozmów, w których inne osoby w Twoim pomieszczeniu mówią do mikrofonu tego komputera. Wiersze mają etykiety Mikrofon sali i Dźwięk rozmowy zamiast Ty i Rozmówcy, a mowa z sali, którą rozmowa powtarza słowo w słowo, pojawia się raz. Od następnego nagrania.", "适用于房间里其他人也对着这台电脑的麦克风说话的通话。各行标为“房间麦克风”和“通话音频”，而不是“你”和“对方”；通话逐字重复的房间发言只显示一次。从下一次录制开始生效。", "同じ部屋のほかの人もこのコンピュータのマイクに向かって話す通話向けです。各行は「あなた」「相手」ではなく「室内マイク」「通話音声」と表示され、通話が一語一句繰り返した室内の発言は一度だけ表示されます。次の録音から有効です。", "Für Anrufe, bei denen andere in deinem Raum ins Mikrofon dieses Computers sprechen. Zeilen heißen Raummikrofon und Anruf-Audio statt Du und Andere, und Sprache aus dem Raum, die der Anruf Wort für Wort wiederholt, erscheint nur einmal. Ab der nächsten Aufnahme.", "Para llamadas en las que otras personas de tu sala hablan al micrófono de este ordenador. Las líneas dicen Micrófono de la sala y Audio de la llamada en lugar de Tú y Ellos, y lo dicho en la sala que la llamada repite palabra por palabra aparece una sola vez. Desde la próxima grabación.", "Для дзвінків, коли інші люди у вашій кімнаті говорять у мікрофон цього комп’ютера. Рядки підписано «Мікрофон кімнати» і «Звук дзвінка» замість «Ви» і «Співрозмовники», а мовлення в кімнаті, яке дзвінок повторює слово в слово, показується один раз. З наступного запису."]),
    ("Room mic", ["Mikrofon sali", "房间麦克风", "室内マイク", "Raummikrofon", "Micrófono de la sala", "Мікрофон кімнати"]),
    ("Call audio", ["Dźwięk rozmowy", "通话音频", "通話音声", "Anruf-Audio", "Audio de la llamada", "Звук дзвінка"]),
    ("Shared room: Room mic is everyone near this computer and Call audio is the call. Hex doesn't tell the people apart.", ["Wspólna sala: Mikrofon sali to wszyscy w pobliżu tego komputera, a Dźwięk rozmowy to rozmowa. Hex nie rozróżnia tych osób.", "共享房间：“房间麦克风”是这台电脑附近的所有人，“通话音频”是通话本身。Hex 不区分其中的人。", "共有ルーム：「室内マイク」はこのコンピュータの近くにいる全員、「通話音声」は通話です。Hex は人を区別しません。", "Gemeinsamer Raum: Raummikrofon sind alle in der Nähe dieses Computers, Anruf-Audio ist der Anruf. Hex unterscheidet die Personen nicht.", "Sala compartida: Micrófono de la sala es todo el que está cerca de este ordenador y Audio de la llamada es la llamada. Hex no distingue a las personas.", "Спільна кімната: «Мікрофон кімнати» — усі поруч із цим комп’ютером, «Звук дзвінка» — сам дзвінок. Hex не розрізняє людей."]),
];
