Feature: A parameterized setup
  Scenario Outline: Checks a caller-supplied email
    Given set variable "receivedEmail" to "<email>"
    Then variable "receivedEmail" should be equal to "test@example.com"

    Examples:
      | email |
      | unused@example.com |
