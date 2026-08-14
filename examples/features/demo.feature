@demo
Feature: demo against a public API

  Scenario: fetching a post
    Given the "Accept" request header is "application/json"
    When I request "/posts/1" using HTTP GET
    Then the response code is 200
    And the response body contains JSON:
      """
      {"id": 1, "userId": "@variableType(int)", "title": "@variableType(string)"}
      """
    And extract "id" from JSON as "postId"
    And variable "postId" should be equal to "1"
