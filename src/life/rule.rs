//! Totalistic B/S rules for the Game of Life, and the names people call them by.
//!
//! ## Why this is a config option and not a twenty-second effect
//!
//! Conway's Life is one rule out of several hundred in the same family, and the
//! crate's own note about a count drifting when nobody checks it against `rg`
//! applies here: shipping `highlife` as its own effect would add a name to
//! `--help`, a section to the config, and a near-identical 1,200 lines to the
//! crate, in exchange for a different pair of numbers. `life` already encodes
//! cell age in its glyph and already has the seeding, the boundary handling and
//! the glider injection, all of which every rule in the family wants. So the
//! family is one option.
//!
//! ## What is and is not here
//!
//! **Totalistic two-state rules only**: a cell is dead or alive, and the rule
//! answers "given a neighbour count, does it live?". That covers the great
//! majority of named Life variants and it is a hard boundary.
//!
//! Two famous rules are *not* in that family and are deliberately absent rather
//! than silently wrong:
//!
//! - **Day & Night** has eight states per cell and follows a one-dimensional
//!   automaton, so it is not expressible as a birth/survival pair at all.
//! - **Replicator** is non-totalistic on the eight-cell neighbourhood: a dead
//!   cell is born from a *particular* arrangement of neighbours, not from a
//!   count, so it needs a 256-bit pattern rather than a nine-bit one.
//!
//! Listing either next to a B/S name would be a lie about what the option
//! accepts, and the failure would be invisible -- the picture would just be the
//! wrong one.
//!
//! ## The notation is the specification
//!
//! A rule is written `B<births>/S<survivals>`, the form every Life reference
//! uses, and it is *parsed* rather than looked up. The named constants below
//! are aliases, not a second definition: `named_rules_are_their_own_b_s_notation`
//! asserts that each name and its B/S string agree, and `a_literal_b_s_string_is
//! _accepted` asserts that the parser handles the whole space, not just the
//! names. So a user can write `rule = "B36/S125"` for a rule nobody has heard
//! of, and there is no table to be out of date.

use serde::Deserialize;
use serde::de::{self, Deserializer};
use std::fmt;

/// A totalistic rule: born on these neighbour counts, survives on those.
///
/// Two nine-bit masks, one bit per count 0 through 8. Counts 0 and 8 are
/// reachable -- a dead cell with 0 live neighbours and a live one with 8 are
/// both ordinary cases -- which is why these are nine bits and not eight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LifeRule {
    birth: u16,
    survive: u16,
}

impl LifeRule {
    /// Conway's Life: born with exactly three, survives on two or three.
    pub const LIFE: LifeRule = LifeRule {
        birth: bit(3),
        survive: bit(2) | bit(3),
    };

    /// The rule a cell is subject to this generation.
    ///
    /// One branch and two shifts, which is the whole cost of making the rule
    /// configurable. It replaced `live_neighbors == 2 || live_neighbors == 3`,
    /// so the simulation is a pair of table lookups slower than it was.
    #[inline]
    pub fn next_state(self, alive: bool, live_neighbors: u8) -> bool {
        let mask = if alive { self.survive } else { self.birth };
        (mask >> live_neighbors) & 1 == 1
    }

    /// The `B../S..` spelling, canonical: digits ascending, no separators.
    pub fn to_spec(self) -> String {
        let mut spec = String::with_capacity(6);
        spec.push('B');
        for n in 0..=8u8 {
            if (self.birth >> n) & 1 == 1 {
                spec.push((b'0' + n) as char);
            }
        }
        spec.push_str("/S");
        for n in 0..=8u8 {
            if (self.survive >> n) & 1 == 1 {
                spec.push((b'0' + n) as char);
            }
        }
        spec
    }

    /// Parses `B3/S23`, or one of the names in [`NAMED_RULES`].
    ///
    /// Case-insensitive, and tolerant of whitespace, because both are things a
    /// person types. Rejects anything else with a message naming what it
    /// accepted, in the same spirit as the crate's refusal to accept a
    /// misspelled terminal colour.
    pub fn parse(text: &str) -> Result<LifeRule, String> {
        let trimmed = text.trim();
        if let Some(rule) = named(trimmed) {
            return Ok(rule);
        }

        let lower = trimmed.to_ascii_lowercase();
        let (birth_part, survive_part) =
            lower.split_once('/').ok_or_else(|| {
                Self::parse_error(
                    text,
                    "there is no '/' between the B and S sections",
                )
            })?;

        // Trimming each half separately, because `B3 / S23` is a thing a person
        // types and rejecting it teaches the user nothing about the real
        // requirement.
        let digits = |part: &str, letter: char| -> Result<u16, String> {
            let part = part.trim();
            let rest = part.strip_prefix(letter).ok_or_else(|| {
                Self::parse_error(
                    text,
                    &format!("the section after '/' does not start with {letter}"),
                )
            })?;
            let mut mask = 0u16;
            for c in rest.chars().filter(|c| !c.is_whitespace()) {
                // `-` is how a reference writes "no counts at all", and an
                // *empty* section means the same thing -- `B2/S` is the standard
                // spelling of `seeds`. Both are accepted, and neither is an
                // error: refusing the empty one would refuse the notation every
                // reference uses for the rule that has no survival counts.
                if c == '-' {
                    continue;
                }
                let n = c.to_digit(10).filter(|d| *d <= 8).ok_or_else(|| {
                    Self::parse_error(
                        text,
                        &format!(
                            "'{c}' is not a neighbour count; counts are 0 to 8"
                        ),
                    )
                })?;
                mask |= bit(n as u8);
            }
            Ok(mask)
        };

        let birth = digits(birth_part, 'b')?;
        let survive = digits(survive_part, 's')?;
        Ok(LifeRule { birth, survive })
    }

    /// The message a bad rule produces.
    ///
    /// It has to carry three things: the **rejected value**, the **reason**, and
    /// the **alternatives**. The value is not decoration -- a user staring at a
    /// config error needs to find which of twenty lines is wrong, and the
    /// alternatives are not written down anywhere else. This is the same shape
    /// as the crate's refusal of a misspelled terminal colour.
    fn parse_error(text: &str, reason: &str) -> String {
        format!(
            "{text:?} is not a Life rule: {reason}. Write one as \
             B<births>/S<survivals> -- \"B3/S23\" is Conway's Life, and \"B2/S\" \
             is Seeds -- or use one of the names: {}",
            named_names().join(", "),
        )
    }
}

impl fmt::Display for LifeRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_spec())
    }
}

impl Default for LifeRule {
    fn default() -> Self {
        LifeRule::LIFE
    }
}

const fn bit(n: u8) -> u16 {
    1 << n
}

/// A named rule and the `B../S..` it stands for.
///
/// The second field is not documentation, it is the assertion: every name here
/// is checked against this spelling in the tests, so a typo cannot turn
/// `highlife` into a different rule that still looks like a plausible Life
/// variant.
pub const NAMED_RULES: &[(&str, &str)] = &[
    ("life", "B3/S23"),
    ("highlife", "B36/S23"),
    ("seeds", "B2/S"),
    ("diamoeba", "B35678/S5678"),
    ("morley", "B368/S245"),
    ("2x2", "B36/S125"),
    ("maze", "B3/S1235"),
    ("coral", "B3/S45678"),
    ("gnarl", "B1/S1"),
    ("longlife", "B3/S12345"),
    ("lotka", "B368/S128"),
    ("34life", "B34/S34"),
    ("life34", "B34/S34"),
    ("mazectric", "B3/S23"),
    ("assimilation", "B345/S4567"),
    ("coagulations", "B378/S378"),
    ("move", "B368/S368"),
    ("periperiodic3", "B12/S12"),
];

fn named(text: &str) -> Option<LifeRule> {
    let lower = text.to_ascii_lowercase();
    NAMED_RULES
        .iter()
        .find(|(name, _)| *name == lower)
        .and_then(|(_, spec)| LifeRule::parse(spec).ok())
}

fn named_names() -> Vec<&'static str> {
    NAMED_RULES.iter().map(|(name, _)| *name).collect()
}

/// Serde hook: rejects an unparseable rule at config-load time.
///
/// This is the same treatment the crate gives a misspelled terminal colour, and
/// for the same reason. A bad *colour* falls back to something visible and
/// obviously wrong; a bad *rule* does not. `rule = "hlghlife"` falling back to
/// Conway's Life would give a perfectly plausible picture of a perfectly
/// ordinary Life, with nothing anywhere saying the config was not read. So this
/// refuses, and the message names what it accepted.
pub fn deserialize_rule<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let text = String::deserialize(deserializer)?;
    LifeRule::parse(&text).map_err(de::Error::custom)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Conway's Life is born on three and survives on two or three.
    ///
    /// The reference every other test is measured against, and the one the whole
    /// feature has to reproduce exactly: a rule table that is subtly wrong would
    /// produce a *Life-like* picture that is not Life, and nothing else in the
    /// crate would notice.
    #[test]
    fn life_is_born_on_three_and_survives_on_two_or_three() {
        let life = LifeRule::LIFE;
        for n in 0..=8u8 {
            assert_eq!(life.next_state(false, n), n == 3, "birth on {n}");
            assert_eq!(
                life.next_state(true, n),
                n == 2 || n == 3,
                "survival on {n}"
            );
        }
    }

    /// The parser accepts the notation directly, not only the names.
    ///
    /// Because that is what makes the family a family: a user can write a rule
    /// nobody has named, and there is no table that can be out of date. If this
    /// were removed and only names worked, every new variant would be a code
    /// change.
    #[test]
    fn a_literal_b_s_string_is_accepted() {
        for spec in [
            "B3/S23",
            "B36/S23",
            "B2/S",
            "B35678/S5678",
            "B368/S245",
            "B36/S125",
            "B3/S1235",
            "B3/S45678",
            "B1/S1",
            "B3/S12345",
            "B368/S128",
            "B34/S34",
        ] {
            let rule =
                LifeRule::parse(spec).unwrap_or_else(|e| panic!("{spec}: {e}"));
            assert_eq!(
                rule.to_spec(),
                spec,
                "{spec} did not survive a round trip through the parser"
            );
        }
    }

    /// Every name is exactly the rule its own `B../S..` says.
    ///
    /// The alias table's second column is the assertion. A mistyped mask there
    /// would give `highlife` a different neighbour count and produce a
    /// perfectly ordinary-looking Life variant, which is the failure mode the
    /// crate's notes about tests that pass against the bug they were written for
    /// are about.
    #[test]
    fn named_rules_are_their_own_b_s_notation() {
        for (name, spec) in NAMED_RULES {
            let by_name = LifeRule::parse(name)
                .unwrap_or_else(|e| panic!("{name} did not resolve: {e}"));
            let by_spec = LifeRule::parse(spec).unwrap_or_else(|e| {
                panic!("{name}'s spec {spec} did not parse: {e}")
            });
            assert_eq!(
                by_name, by_spec,
                "the name {name} resolves to a different rule than its own \
                 notation {spec}"
            );
        }
    }

    /// `highlife` differs from Life in exactly one place.
    ///
    /// Asserted as a single-bit difference rather than as a whole mask, because
    /// HighLife is *defined* as Conway's Life plus a birth on six. A test that
    /// only checked the two masks differ would pass for a rule that had nothing
    /// to do with either.
    #[test]
    fn highlife_is_life_plus_a_birth_on_six() {
        let life = LifeRule::LIFE;
        let highlife = LifeRule::parse("highlife").unwrap();

        assert!(highlife.next_state(false, 6), "the extra birth on six");
        for n in 0..=8u8 {
            if n == 6 {
                continue;
            }
            assert_eq!(
                highlife.next_state(false, n),
                life.next_state(false, n),
                "HighLife should differ from Life only at a birth on six, but \
                 differs at {n}"
            );
        }
        assert_eq!(highlife.survive_mask(), life.survive_mask());
    }

    /// `seeds` dies out, which is the point of it.
    ///
    /// Worth asserting because it is the one named rule that does not merely
    /// *look* different: its whole behaviour is that a soup of isolated pairs
    /// vanishes in a generation or two. A rule table wired up to nothing at all
    /// would look identical to Seeds for the first frame.
    #[test]
    fn seeds_dies_out_and_life_does_not() {
        let seeds = LifeRule::parse("seeds").unwrap();
        assert!(seeds.next_state(false, 2), "Seeds is born on exactly two");
        assert!(
            !seeds.next_state(true, 2) && !seeds.next_state(true, 3),
            "Seeds has no survival counts at all"
        );

        // And the two rules disagree on a single isolated cell, which is the
        // whole difference between them in one case.
        let life = LifeRule::LIFE;
        assert!(!life.next_state(false, 2));
        assert!(seeds.next_state(false, 2));
    }

    /// A blinker oscillates under Life and loses every cell under Seeds.
    ///
    /// The fixture that would catch a table that is correct but *unused*. Every
    /// other test here reads the rule directly, so a step loop still hardcoding
    /// `== 3` would pass all of them and this would fail -- which is the entire
    /// reason this test is at this level rather than in `effect.rs`.
    #[test]
    fn a_blinker_oscillates_under_life_and_loses_every_cell_under_seeds() {
        // A vertical blinker, as a set of (x, y).
        let blink =
            |(cx, cy): (usize, usize), horizontal: bool| -> Vec<(usize, usize)> {
                [-1isize, 0, 1]
                    .iter()
                    .map(|d| {
                        if horizontal {
                            ((cx as isize + d) as usize, cy)
                        } else {
                            (cx, (cy as isize + d) as usize)
                        }
                    })
                    .collect()
            };

        // Applied to a set, using a rule. The whole point is that the *rule*
        // parameter is what decides the outcome.
        let step =
            |cells: &[(usize, usize)], rule: LifeRule| -> Vec<(usize, usize)> {
                let mut next = Vec::new();
                for y in 0..12usize {
                    for x in 0..12usize {
                        let n = cells
                            .iter()
                            .filter(|(cx, cy)| {
                                let dx = (*cx as isize - x as isize).abs();
                                let dy = (*cy as isize - y as isize).abs();
                                (dx <= 1 && dy <= 1) && !(dx == 0 && dy == 0)
                            })
                            .count() as u8;
                        let alive = cells.contains(&(x, y));
                        if rule.next_state(alive, n) {
                            next.push((x, y));
                        }
                    }
                }
                next
            };

        let vertical = blink((6, 6), false);
        let horizontal = blink((6, 6), true);

        assert_eq!(step(&vertical, LifeRule::LIFE), horizontal);
        assert_eq!(step(&horizontal, LifeRule::LIFE), vertical);

        // Seeds has no survival counts, so *every* live cell dies -- which is the
        // definitional difference between the two rules, and the thing an
        // unwired `== 3` hardcode would get wrong.
        //
        // It does not follow that Seeds empties the board. Seeds is born on
        // exactly two, so the four cells diagonally off a blinker each see two
        // live neighbours and are born; the result is a hollow square, not
        // nothing. "Seeds dies out" is a claim about a *dense random soup*, not
        // about every pattern, and an earlier version of this test asserted the
        // blinker case came out empty and was simply wrong.
        let after_seeds = step(&vertical, LifeRule::parse("seeds").unwrap());
        assert!(
            vertical.iter().all(|cell| !after_seeds.contains(cell)),
            "every cell that was live should be dead after a Seeds generation"
        );
        assert!(
            !after_seeds.is_empty(),
            "Seeds is born on two, so a blinker does leave births behind; if \
             this is empty then the birth table is not being consulted"
        );
    }

    /// Bad input is refused, and the message says what was accepted.
    ///
    /// A rule that is quietly wrong gives a plausible Life-like picture, so
    /// silence here is worse than a wrong colour. The message has to carry the
    /// rejected value *and* the alternatives, because the user cannot guess them
    /// and there is no other place they are written down.
    #[test]
    fn a_bad_rule_is_refused_with_the_value_and_the_alternatives() {
        for (bad, expected) in [
            ("hlghlife", "hlghlife"),
            ("B3", "no '/'"),
            ("B3/S2x", "not a neighbour count"),
            ("B9/S23", "not a neighbour count"),
            ("", "no '/'"),
        ] {
            let error = LifeRule::parse(bad).unwrap_err();
            assert!(
                error.contains(expected),
                "parsing {bad:?} should mention {expected:?}, got: {error}"
            );
            assert!(
                error.contains("highlife"),
                "the message for {bad:?} should list the names it accepts, got: \
                 {error}"
            );
        }
    }

    /// An empty survival section is `S-`, not an error.
    ///
    /// Which is how `seeds` is written, and it is the one case where "no
    /// survival counts" is a real rule rather than a typo -- so it needs a
    /// spelling, and `-` is the one every reference uses.
    #[test]
    fn a_dash_means_no_counts() {
        assert_eq!(
            LifeRule::parse("B2/S-").unwrap(),
            LifeRule::parse("B2/S").unwrap()
        );
        assert_eq!(
            LifeRule::parse("B2/S-").unwrap(),
            LifeRule::parse("seeds").unwrap()
        );
    }

    /// Names and specifications are both case-insensitive and tolerate spaces.
    ///
    /// Asserted because both are things a person types, and because a config
    /// that is rejected for `B3 / S23` teaches the user nothing about what went
    /// wrong.
    #[test]
    fn spacing_and_case_do_not_matter() {
        let canonical = LifeRule::LIFE;
        for spelling in [
            "life", "LIFE", "Life", " life ", "B3/S23", "b3/s23", "B3 / S23",
        ] {
            assert_eq!(
                LifeRule::parse(spelling)
                    .unwrap_or_else(|e| panic!("{spelling:?}: {e}")),
                canonical,
                "{spelling:?} should be Conway's Life"
            );
        }
    }

    impl LifeRule {
        /// Test-only view of the survival mask, for the "differs in exactly one
        /// place" assertion.
        fn survive_mask(self) -> u16 {
            self.survive
        }
    }
}
