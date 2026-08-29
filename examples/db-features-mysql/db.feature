@db
Feature: working with the database on MySQL and MariaDB

  Every step below is engine-neutral: this one file passes unchanged against MySQL
  and against MariaDB — only the port in examples/db-mysql.yaml differs. The schema
  is examples/db/init-mysql.sql, spun up via docker-compose.

  What is missing compared with examples/db-features/db.feature is the point of the
  file: no sequences, no procedure or function calls, no search_path. See the
  portability table in examples/README.md for why each of those stays behind.

  Scenario: insert one row, read it back, extract a value
    Given I am in debug mode
    And I have "companies" with "slug: <<unique()>>, name: Acme Inc"
    Then I should have "companies" with "id: <<last_insert_id_companies>>"
    When I extract "name" from "companies" with "id: <<last_insert_id_companies>>" as "acmeName"
    Then variable "acmeName" should be equal to "Acme Inc"
    And I am not in debug mode

  Scenario: a char(36) primary key is filled by bddkit, not by the server
    Given I have "users" with "email: <<unique()>>@example.test"
    Then I should have "users" with "id: <<last_insert_id_users>>"

  Scenario: insert several rows via a table and verify with a table
    Given set variable "slug1" to "<<unique()>>"
    And I have "companies" where:
      | slug         | name   |
      | <<slug1>>    | First  |
      | <<unique()>> | Second |
    Then I should have "companies" with:
      | column | value     |
      | slug   | <<slug1>> |
      | name   | First     |

  Scenario: related tables via a shared "I have:" insert
    Given I have:
      | companies | slug: <<unique()>>, name: Wide Co                                        |
      | invoices  | company_id: <<last_insert_id_companies>>, number: INV-WIDE, amount: 42.50 |
    Then I should have "invoices" with "id: <<last_insert_id_invoices>>"
    And I should have "invoices" with "amount: 42.50"

  Scenario: explicit SQL NULL via <<null>>
    Given I have "companies" with "slug: <<unique()>>, deleted_at: <<null>>"
    Then I should have "companies" with "deleted_at: <<null>>"

  Scenario: update, delete one row, and full cleanup
    Given I have "companies" with "slug: <<unique()>>, name: Temp Co"
    When I update "companies" with "name: Renamed Co" where "id: <<last_insert_id_companies>>"
    Then variable "updated_companies" should be equal to "1"
    And I should have "companies" with "name: Renamed Co"
    When I delete "companies" where "id: <<last_insert_id_companies>>"
    Then variable "deleted_companies" should be equal to "1"
    And I should not have "companies" with "id: <<last_insert_id_companies>>"
    And I should not have "companies" with:
      | column | value                        |
      | id     | <<last_insert_id_companies>> |

    Given set variable "wipeCoSlug" to "<<unique()>>"
    And I have "companies" with "slug: <<wipeCoSlug>>"
    And I extract "id" from "companies" with "slug: <<wipeCoSlug>>" as "wipeCoId"
    And I have "invoices" with "company_id: <<wipeCoId>>, number: INV-WIPE-1, amount: 1"
    And I have "invoices" with "company_id: <<wipeCoId>>, number: INV-WIPE-2, amount: 2"
    When I delete all "invoices"
    Then variable "deleted_invoices" should not be equal to ""
    And I should not have "invoices" with "company_id: <<wipeCoId>>"

  Scenario: switching connection — which here means switching database
    Given I use "reporting" connection
    And I have "audit_log" with "message: demo audit entry"
    Then I should have "audit_log" with "message: demo audit entry"
