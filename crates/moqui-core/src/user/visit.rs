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

//! Visit and visitor rows. Recording writes `moqui.server.Visitor` and `moqui.server.Visit`.

use moqui_lib::object::Value;

use super::login::entity_facade;
use super::session::{self, current_user_id};
use crate::context::Error;
use crate::entity::EntityValue;

const VISITOR: &str = "moqui.server.Visitor";
const VISIT: &str = "moqui.server.Visit";

/// Create a visitor and a visit and remember their ids on the in-memory session.
pub fn record_visit() -> Result<(), Error> {
    let facade = entity_facade()?;
    let mut visitor = facade.make_value(VISITOR)?;
    visitor.set("createdDate", super::login::now_value());
    visitor.set_sequenced_id_primary(&facade)?;
    let visitor_id = sequenced_id(&visitor, "visitorId")?;
    visitor.create(&facade)?;

    let mut visit = facade.make_value(VISIT)?;
    visit.set("visitorId", Value::string(&visitor_id));
    visit.set("fromDate", super::login::now_value());
    if let Some(user_id) = current_user_id()? {
        visit.set("userId", Value::string(user_id));
    }
    visit.set_sequenced_id_primary(&facade)?;
    let visit_id = sequenced_id(&visit, "visitId")?;
    visit.create(&facade)?;
    session::set_visit(&visit_id, &visitor_id);
    Ok(())
}

fn sequenced_id(value: &EntityValue, field: &str) -> Result<String, Error> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .ok_or_else(|| Error::new(format!("{field} was not sequenced")))
}

#[cfg(test)]
mod tests {
    use moqui_lib::object::Value;

    use super::super::login::memdb;
    use super::super::session::{self, current_user_id};
    use super::*;
    use crate::context::ExecutionContextFactory;
    use crate::entity::{EntityFacade, EntityValue};

    #[test]
    fn record_visit_persists_a_visit_and_a_visitor() {
        session::reset_session();
        let ctx = ExecutionContextFactory::test_context();
        let facade = memdb::open();
        ctx.factory().set_entity(facade.clone());
        let _guard = ctx.enter();
        session::set_user("ada", "ada");
        record_visit().unwrap();

        let visit_id = session::visit_id().unwrap().expect("visit id");
        let visitor_id = session::visitor_id().unwrap().expect("visitor id");
        let visit = load(&facade, VISIT, "visitId", &visit_id);
        let visitor = load(&facade, VISITOR, "visitorId", &visitor_id);
        assert_eq!(text(&visit, "visitId"), visit_id);
        assert_eq!(text(&visit, "visitorId"), visitor_id);
        assert_eq!(text(&visitor, "visitorId"), visitor_id);
        assert_eq!(text(&visit, "userId"), "ada");
        assert!(current_user_id().unwrap().is_some());
        session::reset_session();
    }

    #[test]
    fn record_visit_without_a_facade_returns_an_error() {
        session::reset_session();
        let err = record_visit().unwrap_err();
        let shown = err.to_string();
        assert!(
            shown.contains("execution context") || shown.contains("entity facade"),
            "{shown}"
        );
    }

    fn load(facade: &EntityFacade, entity: &str, field: &str, id: &str) -> EntityValue {
        facade
            .find(entity)
            .unwrap()
            .eq(field, Value::string(id))
            .one()
            .unwrap()
            .unwrap_or_else(|| panic!("missing {entity} {id}"))
    }

    fn text(value: &EntityValue, field: &str) -> String {
        value.get(field).and_then(Value::as_str).unwrap_or("").to_string()
    }
}
