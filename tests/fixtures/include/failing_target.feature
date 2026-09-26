Feature: A reusable setup that fails cleanly
  Scenario: Asserts on a variable that was never set
    Then variable "neverSet" should be equal to "anything"
