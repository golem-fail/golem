import XCTest

final class GolemRunnerUITests: XCTestCase {
    static let defaultPort: UInt16 = 8222
    private var server: HTTPServer?

    /// Resolve the port to use: GOLEM_PORT env var, or default 8222.
    private static var resolvedPort: UInt16 {
        if let portStr = ProcessInfo.processInfo.environment["GOLEM_PORT"],
           let port = UInt16(portStr) {
            return port
        }
        return defaultPort
    }

    override func setUp() {
        super.setUp()
        continueAfterFailure = true
        let port = Self.resolvedPort
        let router = RequestRouter()
        let httpServer = HTTPServer(port: port) { method, path, body in
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
