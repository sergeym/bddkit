Feature: Two scenarios, included by name
  Scenario: First
    Given set variable "which" to "first"
  @exports(which)
  Scenario: Second
    Given set variable "which" to "second"
