Feature: A reusable check that must never see the caller's variables
  Scenario: Cannot read the caller's variable
    Then variable "callerOnly" should be empty
