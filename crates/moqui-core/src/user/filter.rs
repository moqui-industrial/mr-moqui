/*
 * This software is in the public domain under CC0 1.0 Universal plus a
 * Grant of Patent License.
 *
 * To the extent possible under law, the author(s) have dedicated all
 * copyright and related and neighboring rights to this software to the
 * public domain worldwide. This software is distributed without any
 * warranty.
 *
 * You should have received a copy of the CC0 Public Domain Dedication
 * along with this software (see the LICENSE.md file). If not, see
 * <http://creativecommons.org/publicdomain/zero/1.0/>.
 */

//! Entity find filters. The condition is returned unchanged until phase 5.

use moqui_db::sql::Condition;

/// Return `condition`. Phase 5 may add `EntityFilter` terms. This does not run a find.
pub fn apply_entity_filters(entity_name: &str, condition: Option<Condition>) -> Option<Condition> {
    let _ = entity_name;
    condition
}

#[cfg(test)]
mod tests {
    use moqui_db::sql::Condition;
    use moqui_lib::object::Value;

    #[test]
    fn apply_entity_filters_returns_the_same_condition() {
        let condition = Condition::equals("testId", Value::string("T1"));
        let filtered = super::apply_entity_filters("moqui.test.TestEntity", Some(condition.clone()));
        assert_eq!(filtered, Some(condition));
        assert!(super::apply_entity_filters("moqui.test.TestEntity", None).is_none());
    }
}
