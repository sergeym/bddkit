Feature: Uses a with: table sourced from a caller variable
  Scenario: with: is interpolated against the caller's scope before the swap
    Given set variable "myEmail" to "test@example.com"
    When I include "../targets/target_with.feature" with:
      | email |
      | <<myEmail>> |
