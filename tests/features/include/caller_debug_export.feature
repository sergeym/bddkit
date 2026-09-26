Feature: Debug mode logs include with/export details
  Scenario: Debug mode displays with and export lines for includes
    Given I am in debug mode
    When I include "outline.feature"
      | value       |
      | debug-value |
    Then variable "seen" should be equal to "debug-value"
