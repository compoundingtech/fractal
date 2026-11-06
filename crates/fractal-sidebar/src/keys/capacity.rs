//! Exact capacity over all canonical key lengths, not independent fixed-width grids.
use std::collections::BTreeMap;
use super::{DIGITS, integer_length};
use fractal_schema::arrangements::valid_key;
use fractal_schema::ordered_membership::MAX_KEY_BYTES as MAX;

// Six bits per character plus one machine word for evenly spaced rank multiplication.
const WORDS: usize = (MAX * 6 + 64) / 64;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
struct Rank([u64; WORDS]);
impl Rank {
    fn small(value: usize) -> Self { let mut rank = Self::default(); rank.0[WORDS - 1] = value as u64; rank }
    fn add(&mut self, other: Self) {
        let mut carry = 0u128;
        for (word, by) in self.0.iter_mut().rev().zip(other.0.iter().rev()) {
            let sum = u128::from(*word) + u128::from(*by) + carry;
            *word = sum as u64; carry = sum >> 64;
        }
        debug_assert_eq!(carry, 0);
    }
    fn sub(&mut self, other: Self) {
        let mut borrow = false;
        for (word, by) in self.0.iter_mut().rev().zip(other.0.iter().rev()) {
            let (value, first) = word.overflowing_sub(*by);
            let (value, second) = value.overflowing_sub(u64::from(borrow));
            *word = value; borrow = first || second;
        }
        debug_assert!(!borrow);
    }
    fn mul(&mut self, by: usize) {
        let mut carry = 0u128;
        for word in self.0.iter_mut().rev() {
            let product = u128::from(*word) * by as u128 + carry;
            *word = product as u64; carry = product >> 64;
        }
        debug_assert_eq!(carry, 0);
    }
    fn div(&mut self, by: usize) {
        let mut remainder = 0u128;
        for word in &mut self.0 {
            let value = (remainder << 64) | u128::from(*word);
            *word = (value / by as u128) as u64; remainder = value % by as u128;
        }
    }
}

struct Trie<'a> {
    lower: Option<&'a str>,
    upper: Option<&'a str>,
    powers: &'a [Rank],
    // Only prefixes intersecting a bound need recursion; interior subtrees have a closed form.
    boundary_counts: BTreeMap<String, Rank>,
}
impl Trie<'_> {
    fn count(&mut self, prefix: &mut String) -> Rank {
        let key = prefix.as_str();
        if self.upper.is_some_and(|hi| key >= hi)
            || self.lower.is_some_and(|lo| key < lo && !lo.starts_with(key)) { return Rank::default(); }
        let max = self.powers.len() - 1;
        let integer = if key.is_empty() { 0 } else { integer_length(key) };
        if integer > max { return Rank::default(); }
        let zero_integer = key.starts_with('A') && key.as_bytes()[1..key.len().min(integer)].iter().all(|b| *b == b'0');
        if zero_integer && key.len() >= integer { return Rank::default(); }
        let after_lower = self.lower.is_none_or(|lo| key > lo);
        let valid = !key.is_empty() && valid_key(key);
        if !key.is_empty() && after_lower && self.upper.is_none_or(|hi| !hi.starts_with(key)) {
            let mut count = self.powers[max - key.len()];
            // One complete integer owns exactly 62^(remaining bytes) canonical fractions,
            // including its empty fraction. A fractional prefix ending in zero is not a key.
            if zero_integer { count.sub(self.powers[max - integer]); }
            else if key.len() >= integer && !valid { count.sub(Rank::small(1)); }
            return count;
        }
        if let Some(count) = self.boundary_counts.get(key) { return *count; }
        let mut total = Rank::small(usize::from(valid && after_lower));
        if key.len() < max {
            let alphabet = if key.is_empty() { &DIGITS[10..] } else { DIGITS };
            for byte in alphabet {
                prefix.push(char::from(*byte));
                total.add(self.count(prefix));
                prefix.pop();
            }
        }
        self.boundary_counts.insert(prefix.clone(), total);
        total
    }
    fn select(&mut self, prefix: &mut String, mut rank: Rank) -> String {
        if valid_key(prefix) && self.lower.is_none_or(|lo| prefix.as_str() > lo) {
            if rank == Rank::default() { return prefix.clone(); }
            rank.sub(Rank::small(1));
        }
        let alphabet = if prefix.is_empty() { &DIGITS[10..] } else { DIGITS };
        for byte in alphabet {
            prefix.push(char::from(*byte));
            let capacity = self.count(prefix);
            if rank < capacity { return self.select(prefix, rank); }
            rank.sub(capacity);
            prefix.pop();
        }
        unreachable!("rank was admitted against exact canonical capacity")
    }
}

/// Allocate `count` canonical keys strictly inside the bounds. Use the shortest sufficient
/// maximum length, selecting evenly spaced ranks over its union of all shorter legal lengths.
/// None means invalid bounds or insufficient capacity even through MAX_KEY_BYTES.
pub fn spread(lower: Option<&str>, upper: Option<&str>, count: usize) -> Option<Vec<String>> {
    spread_with_max(lower, upper, count, MAX)
}
fn spread_with_max(lower: Option<&str>, upper: Option<&str>, count: usize, max: usize) -> Option<Vec<String>> {
    if lower.into_iter().chain(upper).any(|key| !valid_key(key))
        || lower.zip(upper).is_some_and(|(lo, hi)| lo >= hi) { return None; }
    if count == 0 { return Some(Vec::new()); }
    let denominator = count.checked_add(1)?;
    let mut powers = [Rank::default(); MAX + 1];
    powers[0] = Rank::small(1);
    for index in 1..=max { powers[index] = powers[index - 1]; powers[index].mul(62); }
    let mut prefix = String::with_capacity(max);
    for horizon in 2..=max {
        let mut trie = Trie { lower, upper, powers: &powers[..=horizon], boundary_counts: BTreeMap::new() };
        let total = trie.count(&mut prefix);
        if total < Rank::small(count) { continue; }
        let mut keys = Vec::with_capacity(count);
        for index in 1..=count {
            let mut rank = total;
            rank.mul(index); rank.div(denominator);
            prefix.clear();
            keys.push(trie.select(&mut prefix, rank));
        }
        return Some(keys);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_max_length_gap_has_seven_slots_without_rekeying_the_upper_bound() {
        let prefix = format!("a0{}", "0".repeat(MAX - 3));
        let lower = format!("{prefix}1"); let upper = format!("{prefix}z");
        let keys = spread(Some(&lower), Some(&upper), 7).unwrap();
        assert_eq!(keys.len(), 7);
        assert!(keys.iter().all(|key| valid_key(key) && lower < *key && *key < upper));
        assert!(keys.windows(2).all(|pair| pair[0] < pair[1]));
    }
    #[test]
    fn capacity_includes_a_short_prefix_and_its_long_descendant() {
        let lower = format!("a0V{}", "z".repeat(MAX - 3));
        let upper = format!("a0W{}2", "0".repeat(MAX - 4));
        let descendant = format!("a0W{}1", "0".repeat(MAX - 4));
        assert_eq!(spread(Some(&lower), Some(&upper), 2).unwrap(), ["a0W".to_owned(), descendant]);
        assert!(spread(Some(&lower), Some(&upper), 3).is_none());
    }
    #[test]
    fn exact_capacity_and_even_ranks_match_an_independent_small_universe() {
        let mut universe = Vec::new();
        for head in b"YZab" {
            for first in DIGITS {
                let prefix = format!("{}{}", char::from(*head), char::from(*first));
                if valid_key(&prefix) { universe.push(prefix.clone()); }
                for last in DIGITS {
                    let key = format!("{prefix}{}", char::from(*last));
                    if valid_key(&key) { universe.push(key); }
                }
            }
        }
        universe.sort();
        for start in [0, 60, 61, 122, 1000, universe.len() - 64] {
            let lower = &universe[start]; let upper = &universe[start + 62];
            let available = &universe[start + 1..start + 62];
            for count in [0, 1, 2, 5, 17, 61, 62] {
                let actual = spread_with_max(Some(lower), Some(upper), count, 3);
                if count > available.len() { assert!(actual.is_none()); }
                else {
                    let short: Vec<_> = available.iter().filter(|key| key.len() == 2).collect();
                    let pool: Vec<_> = if short.len() >= count { short } else { available.iter().collect() };
                    assert_eq!(actual.unwrap(), (1..=count).map(|i| pool[pool.len() * i / (count + 1)].clone()).collect::<Vec<_>>());
                }
            }
        }
        for count in [1, 2, 17] {
            let short: Vec<_> = universe.iter().filter(|key| key.len() == 2).collect();
            assert_eq!(spread_with_max(None, None, count, 3).unwrap(), (1..=count).map(|i| short[short.len() * i / (count + 1)].clone()).collect::<Vec<_>>());
        }
    }
    #[test]
    fn invalid_and_full_gaps_are_explicit() {
        assert!(spread(Some("invalid"), None, 1).is_none());
        assert!(spread(Some("a0"), Some("a0"), 1).is_none());
        assert!(spread(None, None, usize::MAX).is_none());
        assert_eq!(spread(None, None, 0), Some(Vec::new()));
    }
}
