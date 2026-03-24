import XCTest

final class GolemRunnerUITests: XCTestCase {
    static let defaultPort: UInt16 = 8222
    private var server: HTTPServer?

    override func setUp() {
        super.setUp()
        continueAfterFailure = true
        let router = RequestRouter()
        let httpServer = HTTPServer(port: Self.defaultPort) { method, path, body in
            return router.handle(method: method, path: path, body: body)
        }
        do {
            try httpServer.start()
            server = httpServer
        } catch {
            XCTFail("Failed to start HTTP server: \(error)")
        }
    }

    override func tearDown() {
        server?.stop()
        super.tearDown()
    }

    func testCompanionServer() {
        let keepAlive = expectation(description: "Server runs until killed")
        keepAlive.isInverted = true
        wait(for: [keepAlive], timeout: .infinity)
    }
}
