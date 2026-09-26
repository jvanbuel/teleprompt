import Foundation

/// The repository root, from this file's place in it.
let repository = URL(fileURLWithPath: #filePath)
    .deletingLastPathComponent() // TelepromptKitTests
    .deletingLastPathComponent() // Tests
    .deletingLastPathComponent() // macos
    .deletingLastPathComponent() // apps
    .deletingLastPathComponent()

/// An example from `docs/api/v1/examples`, which the server is tested
/// against too.
func example(_ name: String) throws -> Data {
    try Data(contentsOf: repository.appendingPathComponent("docs/api/v1/examples/\(name)"))
}

/// JSON as a comparable value, so key order and spacing do not matter.
func jsonObject(_ data: Data) throws -> NSObject {
    try JSONSerialization.jsonObject(with: data) as! NSObject
}
