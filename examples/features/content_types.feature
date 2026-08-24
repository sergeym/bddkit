@demo @content-types
Feature: responses that are not JSON

  # Note: there is no assertion on a raw (non-JSON) body yet (issue #4), so
  # these scenarios pin the status code and the Content-Type, and print the
  # body for a human to read.

  Scenario Outline: the body comes back with the content type it claims
    Given the "Accept" request header is "<accept>"
    When I request "<path>" using HTTP GET
    Then the response code is 200
    And the "Content-Type" response header is "<content_type>"
    And Print response body

    # <placeholder> is filled in when the file is parsed, once per row —
    # a different syntax from the runtime <<variable>>, on purpose.
    Examples:
      | path          | accept          | content_type              |
      | /posts/1.html | text/html       | text/html; charset=utf-8  |
      | /posts/1.xml  | application/xml | application/xml           |
      | /health       | text/plain      | text/plain; charset=utf-8 |

  Scenario: a form login hands back a session cookie
    Given the request form parameters are:
      | name     | value  |
      | user     | demo   |
      | password | s3cr3t |
    When I request "/login" using HTTP POST
    Then the response code is 200
    And the response body contains JSON:
      """
      {"status": "ok"}
      """
    And extract "session" from cookies as "sessionId"
    And variable "sessionId" should be equal to "s3cr3t-session"
