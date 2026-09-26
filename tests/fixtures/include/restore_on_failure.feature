Feature: The caller's variables survive a failed include
  Scenario: The included scenario fails
    Given set variable "before" to "kept"
    Then I include "../targets/failing_target.feature"

  Scenario: The file-level VarStack was restored, not left swapped
    Then variable "before" should be equal to "kept"
