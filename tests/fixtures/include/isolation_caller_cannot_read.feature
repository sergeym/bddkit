Feature: The caller's variable is invisible inside the included scenario
  Scenario: Caller sets a variable then includes a scenario that must not see it
    Given set variable "callerOnly" to "secret"
    Then I include "../targets/isolation_target_cannot_read_caller.feature"
