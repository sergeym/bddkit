Feature: A reusable setup that declares an export it never sets
  @exports(neverSet)
  Scenario: Declares an export that never gets set
    Given set variable "unrelated" to "x"
