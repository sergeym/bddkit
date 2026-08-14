@vars
Feature: variables

  Scenario: setting a variable and substituting it into a path
    Given set variable "path" to "/ping"
    And the "Accept" request header is "application/json"
    When I request "<<path>>" using HTTP GET
    Then the response code is 200

  Scenario: extracting from JSON and comparing
    When I request "/ping" using HTTP GET
    Then extract "version" from JSON as "v"
    And variable "v" should be equal to "3"
    And variable "v" should not be equal to "4"

  Scenario: extracting from cookies
    Given the "Content-Type" request header is "application/json"
    And the request body is:
      """
      {"password": "correct"}
      """
    When I request "/login" using HTTP POST
    Then extract "jwt_token" from cookies as "token"
    And variable "token" should be equal to "tok-abc"

  Scenario: unique values differ
    Given set variable "a" to "<<unique(token)>>"
    And set variable "b" to "<<unique(token)>>"
    Then variable "a" should not be equal to "<<b>>"

  Scenario: variable survives the scenario boundary
    Then variable "a" should not be equal to ""
