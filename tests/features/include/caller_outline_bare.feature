Feature: Includes an outline with the lone Examples row
  Scenario: Includes outline without with: and sees its export
    Given I include "outline.feature"
    Then variable "seen" should be equal to "placeholder"
