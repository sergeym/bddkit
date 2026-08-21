@demo
Feature: demo against a public API

  Scenario: fetch a post
    Given the "Accept" request header is "application/json"
    When I request "/posts/1" using HTTP GET
    Then the response code is 200
    And the response body contains JSON:
      """
      {"id": 1, "userId": "@variableType(int)", "title": "@variableType(string)"}
      """
    And extract "id" from JSON as "postId"
    And variable "postId" should be equal to "1"
    And Print response headers
    And Print response body
    And Print response body as "title"

  Scenario: create a post via a macro
    When I create a post with title "new post"
    Then variable "lastPostId" should be equal to "101"
