//! Picking a Windows audio endpoint by part of its name.

/// Which of the active endpoints' `names` the user meant by `wanted`: the one
/// whose name contains it, ignoring case. None or several is an error that lists
/// the names, so the user can pick.
#[cfg_attr(
    all(not(windows), not(test)),
    expect(dead_code, reason = "measure is Windows-only (WASAPI)")
)]
pub fn pick_endpoint(names: &[String], wanted: &str) -> Result<usize, String> {
    let needle = wanted.to_lowercase();
    let matches: Vec<usize> = (0..names.len())
        .filter(|&i| names[i].to_lowercase().contains(&needle))
        .collect();
    let quoted = |indexes: &mut dyn Iterator<Item = usize>| {
        indexes
            .map(|i| format!("\"{}\"", names[i]))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match matches[..] {
        [one] => Ok(one),
        [] => Err(format!(
            "no active endpoint's name contains \"{wanted}\"; there are: {}",
            quoted(&mut (0..names.len()))
        )),
        _ => Err(format!(
            "\"{wanted}\" is in more than one endpoint's name: {}; be more specific",
            quoted(&mut matches.iter().copied())
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_is_picked_by_part_of_its_name_ignoring_case() {
        let names = [
            "Speakers (Realtek Audio)",
            "CABLE Input (VB-Audio Virtual Cable)",
        ]
        .map(String::from);

        assert_eq!(pick_endpoint(&names, "cable input"), Ok(1));
    }

    #[test]
    fn no_match_or_several_matches_name_what_there_is() {
        let names = ["CABLE Input (VB-Audio Virtual Cable)", "CABLE-A Input"].map(String::from);

        assert_eq!(
            pick_endpoint(&names, "Voicemeeter"),
            Err(
                "no active endpoint's name contains \"Voicemeeter\"; there are: \
                 \"CABLE Input (VB-Audio Virtual Cable)\", \"CABLE-A Input\""
                    .to_string()
            )
        );
        assert_eq!(
            pick_endpoint(&names, "input"),
            Err("\"input\" is in more than one endpoint's name: \
                 \"CABLE Input (VB-Audio Virtual Cable)\", \"CABLE-A Input\"; be more specific"
                .to_string())
        );
    }
}
