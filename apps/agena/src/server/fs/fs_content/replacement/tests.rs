use super::ReplacementPlan;

#[test]
fn bounded_plan_preserves_the_regex_library_replacement_grammar() {
    let regex = regex::Regex::new(r"(a)(?P<number>[0-9])?(?P<tail>b*)").unwrap();
    let content = "a1bb a abbb a9b";
    for replacement in [
        "$0",
        "$1$2",
        "${number}-${tail}",
        "$$",
        "$",
        "${}",
        "${missing}",
        "$123456789012345678901234567890",
        "$number_tail",
        "${number}_tail",
        "${尾巴}",
        "$$$1${number}$$${tail}",
        "${1}$2-${3}",
        "${",
        "$!",
        "one\ntwo",
    ] {
        let expected = regex.replace_all(content, replacement);
        let plan = ReplacementPlan::new(regex.clone(), replacement.to_owned(), true);
        let (actual, count) = plan.apply(content).unwrap();
        assert_eq!(actual, expected, "{replacement:?}");
        assert_eq!(count, regex.find_iter(content).count());
    }
}

#[test]
fn literal_plans_preserve_replacement_text_and_zero_width_match_positions() {
    for pattern in ["a", r"\b", r"a|z", "xyz"] {
        let regex = regex::Regex::new(pattern).unwrap();
        for replacement in ["$0", "$$", "${word}", "é😀", ""] {
            let content = "a and éa";
            let expected = regex.replace_all(content, regex::NoExpand(replacement));
            let plan = ReplacementPlan::new(regex.clone(), replacement.to_owned(), false);
            let (actual, count) = plan.apply(content).unwrap();
            assert_eq!(actual, expected, "{pattern:?}: {replacement:?}");
            assert_eq!(count, regex.find_iter(content).count());
        }
    }
}
