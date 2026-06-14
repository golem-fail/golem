import Foundation
import Testing

/// Unit tests for `HTTPResponse`'s pure value builders. These run in a logic
/// unit-test bundle — no simulator UI automation involved.
struct HTTPResponseTests {
    @Test func jsonSetsStatusContentTypeAndSortsKeys() {
        let r = HTTPResponse.json(["b": 2, "a": 1])
        #expect(r.statusCode == 200)
        #expect(r.statusText == "OK")
        #expect(r.contentType == "application/json")
        // `.sortedKeys` → "a" emitted before "b".
        #expect(String(data: r.body, encoding: .utf8) == #"{"a":1,"b":2}"#)
    }

    @Test func jsonHonorsExplicitStatus() {
        let r = HTTPResponse.json(["ok": true], status: 404)
        #expect(r.statusCode == 404)
        #expect(r.statusText == "Not Found")
    }

    @Test func errorWrapsMessageAsJSON() throws {
        let r = HTTPResponse.error("boom", status: 400)
        #expect(r.statusCode == 400)
        #expect(r.statusText == "Bad Request")
        #expect(r.contentType == "application/json")
        let obj = try JSONSerialization.jsonObject(with: r.body) as? [String: String]
        #expect(obj?["error"] == "boom")
    }

    @Test func pngUsesImageContentType() {
        let bytes = Data([0x89, 0x50, 0x4E, 0x47])
        let r = HTTPResponse.png(bytes)
        #expect(r.statusCode == 200)
        #expect(r.statusText == "OK")
        #expect(r.contentType == "image/png")
        #expect(r.body == bytes)
    }

    @Test func jsonDataPassesBytesThrough() {
        let raw = Data(#"{"x":1}"#.utf8)
        let r = HTTPResponse.jsonData(raw, status: 500)
        #expect(r.statusCode == 500)
        #expect(r.statusText == "Internal Server Error")
        #expect(r.body == raw)
    }

    @Test(arguments: [
        (code: 200, text: "OK"),
        (code: 400, text: "Bad Request"),
        (code: 404, text: "Not Found"),
        (code: 500, text: "Internal Server Error"),
        (code: 418, text: "Unknown"),
    ])
    func statusTextMapping(_ c: (code: Int, text: String)) {
        // `statusText(for:)` is private; exercise it through a builder.
        #expect(HTTPResponse.jsonData(Data(), status: c.code).statusText == c.text)
    }
}
