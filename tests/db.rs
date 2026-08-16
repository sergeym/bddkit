mod common;

use common::db::{combined, run_feature, setup};

// The fixture schema is shared, so DB tests run on one thread (--test-threads=1).

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_identity_and_uuid_pk() {
    let _g = setup().await;
    // companies: the identity PK is omitted; users: the uuid PK is client-generated,
    // created_at NOT NULL with no default gets filled with now(). Both rows are read
    // back by the generated PK — so last_insert_id_* were set correctly.
    let src = "\
Feature: insert
  Scenario: identity and uuid
    Given I have \"companies\" with \"slug: acme\"
    Then I should have \"companies\" with \"id: <<last_insert_id_companies>>\"
    Given I have \"users\" with \"email: a@b.net\"
    Then I should have \"users\" with \"id: <<last_insert_id_users>>\"
";
    let out = run_feature(src);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(combined(&out).contains("failures: 0"), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_not_null_without_default_fails_naming_column() {
    let _g = setup().await;
    // companies.slug is NOT NULL with no default and no value → error names the column.
    let src = "\
Feature: insert
  Scenario: missing slug
    Given I have \"companies\" with \"name: no-slug\"
";
    let out = run_feature(src);
    assert!(!out.status.success(), "must fail: {}", combined(&out));
    assert!(combined(&out).contains("slug"), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wide_table_insert_sets_indexed_vars() {
    let _g = setup().await;
    let src = "\
Feature: insert
  Scenario: table form
    Given I have \"companies\" where:
      | slug   |
      | first  |
      | second |
    Then I should have \"companies\" with \"id: <<last_insert_id_companies_0>>\"
    And I should have \"companies\" with \"id: <<last_insert_id_companies_1>>\"
";
    let out = run_feature(src);
    assert!(out.status.success(), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_sets_updated_counter() {
    let _g = setup().await;
    // updated_companies == 1 after UPDATE; the last step looks for the now-deleted
    // row and must fail (I should not have arrives in Task 10).
    let src = "\
Feature: mutate
  Scenario: update counter
    Given I have \"companies\" with \"slug: acme\"
    When I update \"companies\" with \"name: Renamed\" where \"id: <<last_insert_id_companies>>\"
    Then variable \"updated_companies\" should be equal to \"1\"
    When I delete \"companies\" where \"id: <<last_insert_id_companies>>\"
    Then I should have \"companies\" with \"slug: acme\"
";
    let out = run_feature(src);
    assert!(!out.status.success(), "a deleted row must not be found: {}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_all_empties_table() {
    let _g = setup().await;
    let src = "\
Feature: mutate
  Scenario: wipe table
    Given I have \"companies\" with \"slug: one\"
    And I have \"companies\" with \"slug: two\"
    When I delete all \"companies\"
    Then I should have \"companies\" with \"slug: one\"
";
    // after DELETE ALL the table is empty — the row lookup must fail.
    let out = run_feature(src);
    assert!(!out.status.success(), "the table must be empty: {}", combined(&out));
}
