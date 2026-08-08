/// Znake s posebnim pomenom v HTML-ju zamenja z varnimi entitetami.
///
/// Funkcijo uporabimo pred vstavljanjem uporabniških podatkov v
/// strežniško ustvarjene HTML fragmente.
pub(crate) fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
