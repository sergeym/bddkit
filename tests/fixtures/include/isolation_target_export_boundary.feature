Feature: A reusable setup that exports one variable and sets an unexported one
  @exports(kept)
  Scenario: Declares one export and sets one undeclared variable
    Given set variable "kept" to "yes"
    And set variable "notExported" to "no"
