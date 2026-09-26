Feature: Only the declared export crosses back to the caller
  Scenario: Includes and captures the declared export
    Given I include "../targets/isolation_target_export_boundary.feature"
    Then variable "kept" should be equal to "yes"

  Scenario: The undeclared variable never arrives
    Then variable "notExported" should be empty
