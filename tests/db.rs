mod common;

use common::db::{combined, run_feature, setup};

// The fixture schema is shared, so DB tests run on one thread (--test-threads=1).

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn insert_identity_and_uuid_pk() {
    let _g = setup().await;
    // companies: identity PK is omitted; users: uuid PK is generated client-side,
    // created_at NOT NULL without a default is filled with now(). Both rows are read
    // back by the generated PK — meaning last_insert_id_* are set.
    let src = "\
Feature: insert
  Scenario: identity and uuid
    Given I have \"companies\" with \"slug: acme\"
    Then I should have \"companies\" with \"id: <<last_insert_id_companies>>\"
    Given I have \"users\" with \"email: a@b.net\"
    Then I should have \"users\" with \"id: <<last_insert_id_users>>\"
";
    let out = run_feature(src, &_g);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(combined(&out).contains("failed: 0"), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_not_null_without_default_fails_naming_column() {
    let _g = setup().await;
    // companies.slug NOT NULL without a default and without a value → error naming the column.
    let src = "\
Feature: insert
  Scenario: missing slug
    Given I have \"companies\" with \"name: no-slug\"
";
    let out = run_feature(src, &_g);
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
    let out = run_feature(src, &_g);
    assert!(out.status.success(), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn update_sets_updated_counter() {
    let _g = setup().await;
    // updated_companies == 1 after UPDATE; the last step looks for the already-deleted
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
    let out = run_feature(src, &_g);
    assert!(!out.status.success(), "a deleted record must not be found: {}", combined(&out));
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
    // after DELETE ALL the table is empty — looking up a row must fail.
    let out = run_feature(src, &_g);
    assert!(!out.status.success(), "table must be empty: {}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn should_not_have_passes_for_absent_row() {
    let _g = setup().await;
    // A row with slug: acme exists; negating the presence of a nonexistent slug passes.
    let src = "\
Feature: absence
  Scenario: negative existence ok
    Given I have \"companies\" with \"slug: acme\"
    Then I should not have \"companies\" with \"slug: ghost\"
";
    let out = run_feature(src, &_g);
    assert!(out.status.success(), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn should_not_have_fails_for_present_row() {
    let _g = setup().await;
    // Negating the presence of an existing row must fail.
    let src = "\
Feature: absence
  Scenario: negative existence violated
    Given I have \"companies\" with \"slug: acme\"
    Then I should not have \"companies\" with \"slug: acme\"
";
    let out = run_feature(src, &_g);
    assert!(!out.status.success(), "an existing record must violate the negation: {}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn null_sentinel_insert_matches_is_null() {
    let _g = setup().await;
    // deleted_at is inserted as <<null>> (SQL NULL); looking up by <<null>> uses IS NULL.
    let src = "\
Feature: null
  Scenario: insert null and match is null
    Given I have \"users\" with \"email: a@b.net, deleted_at: <<null>>\"
    Then I should have \"users\" with \"deleted_at: <<null>>\"
";
    let out = run_feature(src, &_g);
    assert!(out.status.success(), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn composite_pk_insert_reads_back_via_last_insert_vars() {
    let _g = setup().await;
    // pair(a, b) is a composite PK; RETURNING stores last_insert_pair_a / _b,
    // which are used to read the row back.
    let src = "\
Feature: composite pk
  Scenario: insert and read back pair
    Given I have \"pair\" with \"a: 1, b: 2, note: linked\"
    Then I should have \"pair\" with \"a: <<last_insert_pair_a>>\"
    And I should have \"pair\" with \"b: <<last_insert_pair_b>>\"
";
    let out = run_feature(src, &_g);
    assert!(out.status.success(), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delete_all_sets_deleted_counter() {
    let _g = setup().await;
    // deleted_companies == 2 after DELETE ALL on two rows.
    let src = "\
Feature: mutate
  Scenario: delete all counter
    Given I have \"companies\" with \"slug: one\"
    And I have \"companies\" with \"slug: two\"
    When I delete all \"companies\"
    Then variable \"deleted_companies\" should be equal to \"2\"
";
    let out = run_feature(src, &_g);
    assert!(out.status.success(), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extract_from_table_binds_variable() {
    let _g = setup().await;
    // extract stores the column value in a variable; the next step checks
    // it via <<cid>> — a match confirms extract bound the variable.
    let src = "\
Feature: extract
  Scenario: pull id into var
    Given I have \"companies\" with \"slug: acme\"
    When I extract \"id\" from \"companies\" with \"slug: acme\" as \"cid\"
    Then I should have \"companies\" with \"id: <<cid>>\"
";
    let out = run_feature(src, &_g);
    assert!(out.status.success(), "{}", combined(&out));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sequence_and_builtin_function() {
    let _g = setup().await;
    // nextval increases; upper() is a builtin function with a text argument.
    let src = "\
Feature: routines
  Scenario: sequence and function
    Given I get next value of sequence \"apibdd_it.thing_seq\" as \"first\"
    And I get next value of sequence \"apibdd_it.thing_seq\" as \"second\"
    Then variable \"second\" should not be equal to \"<<first>>\"
    When I call function \"upper\" with \"s: abc\" as \"up\"
    Then variable \"up\" should be equal to \"ABC\"
";
    let out = run_feature(src, &_g);
    assert!(out.status.success(), "{}", combined(&out));
}
