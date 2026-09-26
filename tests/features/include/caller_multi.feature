Feature: Includes a named scenario from a multi-scenario file
  Scenario: Includes Second scenario and sees its export
    Given I include "multi.feature" scenario "Second"
    Then variable "which" should be equal to "second"
