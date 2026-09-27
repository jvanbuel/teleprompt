import Foundation

/// A run of words in a line against what its take said.
public enum SaidChange: Equatable, Sendable {
    case same(String)
    /// In the line, not said.
    case gone(String)
    /// Said, not in the line.
    case new(String)
}

/// `line` against `said`, by their longest common run of words; in a gap,
/// what goes before what comes. As the Linux app's `said::diff`.
public func saidDiff(_ line: String, _ said: String) -> [SaidChange] {
    let a = line.split(whereSeparator: { $0.isWhitespace }).map(String.init)
    let b = said.split(whereSeparator: { $0.isWhitespace }).map(String.init)
    var common = Array(repeating: Array(repeating: 0, count: b.count + 1), count: a.count + 1)
    for i in stride(from: a.count - 1, through: 0, by: -1) {
        for j in stride(from: b.count - 1, through: 0, by: -1) {
            common[i][j] = a[i] == b[j] ? common[i + 1][j + 1] + 1 : max(common[i + 1][j], common[i][j + 1])
        }
    }
    var out: [SaidChange] = []
    func push(_ change: SaidChange) {
        switch (out.last, change) {
        case let (.same(run)?, .same(w)): out[out.count - 1] = .same(run + " " + w)
        case let (.gone(run)?, .gone(w)): out[out.count - 1] = .gone(run + " " + w)
        case let (.new(run)?, .new(w)): out[out.count - 1] = .new(run + " " + w)
        default: out.append(change)
        }
    }
    var (i, j) = (0, 0)
    while i < a.count || j < b.count {
        if i < a.count, j < b.count, a[i] == b[j] {
            push(.same(a[i]))
            i += 1
            j += 1
        } else if i < a.count, j == b.count || common[i + 1][j] >= common[i][j + 1] {
            push(.gone(a[i]))
            i += 1
        } else {
            push(.new(b[j]))
            j += 1
        }
    }
    return out
}
