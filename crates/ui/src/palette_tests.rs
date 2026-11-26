use super::*;
use crate::registry;

#[test]
fn folding_and_word_scores() {
    assert_eq!(fold("Modèle › Relations"), "modele   relations");
    assert_eq!(fold("measure.add"), "measure add");
    // a substring beats letters in order; the start of a word beats the middle
    let sub = score_word("rel", "new relationship").unwrap();
    let mid = score_word("ati", "new relationship").unwrap();
    let seq = score_word("nrl", "new relationship").unwrap();
    assert!(sub > mid && mid > seq, "{sub} {mid} {seq}");
    assert!(score_word("xyz", "new relationship").is_none());
    assert_eq!(score_word("", "x"), Some(0));
}

/// Matches on the label, the id and the place, in English and French, whichever the window's
/// language; every word must match.
#[test]
fn commands_match_on_label_id_and_place_in_both_languages() {
    let rel = registry::find("relationship.add").unwrap();
    let measure = registry::find("measure.add").unwrap();
    for lang in [Lang::En, Lang::Fr] {
        assert!(score("new relationship", rel, lang).is_some());
        assert!(score("nouvelle relation", rel, lang).is_some());
        assert!(score("relationship.add", rel, lang).is_some());
        assert!(
            score("modele relations", rel, lang).is_some(),
            "the place, without accents"
        );
        assert!(score("msr add", measure, lang).is_some(), "letters in order");
        assert!(score("new zebra", rel, lang).is_none());
    }
    // the better match ranks higher
    let page = registry::find("page.add").unwrap();
    let arrange = registry::find("page.arrange").unwrap();
    assert!(score("new page", page, Lang::En) > score("new page", arrange, Lang::En));
}
