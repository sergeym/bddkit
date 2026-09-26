Feature: Includes an outline with a caller-supplied row
  Scenario: Includes outline with a with: table and sees its export
    When I include "outline.feature"
      | value       |
      | from-caller |
    Then variable "seen" should be equal to "from-caller"
