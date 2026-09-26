@demo @include
Feature: reusable setup scenarios, meant to be pulled in with `I include`

  # A plain scenario, included by name. @exports(userId) is what lets
  # "userId" survive back to the caller — "internalOnly" does not, since
  # it is never named in the tag.
  @exports(userId)
  Scenario: a user signs up
    Given set variable "userId" to "abc123"
    And set variable "internalOnly" to "not exported"

  # A Scenario Outline, meant to be included with a `with:` table so the
  # caller supplies the row instead of the placeholder Examples value below.
  @exports(seen)
  Scenario Outline: records what it was given
    Given set variable "seen" to "<value>"

    Examples:
      | value       |
      | placeholder |
