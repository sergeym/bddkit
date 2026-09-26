Feature: Uses the reusable setup
  Scenario: Includes a one-scenario file and sees its export
    Given I include "target.feature"
    Then variable "userId" should be equal to "abc123"
