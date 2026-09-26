Feature: An outline included with a caller-supplied row
  @exports(seen)
  Scenario Outline: Records what it was given
    Given set variable "seen" to "<value>"
    Examples:
      | value       |
      | placeholder |
