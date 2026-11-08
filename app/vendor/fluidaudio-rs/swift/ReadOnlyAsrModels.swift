import Foundation
import CoreML
import FluidAudio

/// Unlike the SDK's load/loadFromCache/downloadAndLoad helpers, this path has
/// no recovery, removal, directory creation, compilation or network operation.
/// It accepts only the compiled Parakeet v3 directory resolved by Murmur.
enum ReadOnlyAsrModels {
    static func load(from directory: URL) throws -> AsrModels {
        let config = AsrModels.defaultConfiguration()
        let preprocessorConfig = MLModelConfiguration()
        preprocessorConfig.computeUnits = .cpuOnly
        let preprocessor = try MLModel(
            contentsOf: directory.appendingPathComponent("Preprocessor.mlmodelc"),
            configuration: preprocessorConfig)
        let encoder = try MLModel(
            contentsOf: directory.appendingPathComponent("Encoder.mlmodelc"), configuration: config)
        let decoder = try MLModel(
            contentsOf: directory.appendingPathComponent("Decoder.mlmodelc"), configuration: config)
        let joint = try MLModel(
            contentsOf: directory.appendingPathComponent("JointDecisionv3.mlmodelc"), configuration: config)
        let data = try Data(contentsOf: directory.appendingPathComponent("parakeet_vocab.json"))
        let json = try JSONSerialization.jsonObject(with: data)
        let vocabulary: [Int: String]
        if let tokens = json as? [String] {
            vocabulary = Dictionary(uniqueKeysWithValues: tokens.enumerated().map { ($0.offset, $0.element) })
        } else if let tokens = json as? [String: String] {
            var parsed: [Int: String] = [:]
            for (key, value) in tokens {
                guard let id = Int(key), id >= 0, parsed[id] == nil else {
                    throw CocoaError(.fileReadCorruptFile)
                }
                parsed[id] = value
            }
            vocabulary = parsed
        } else {
            throw CocoaError(.fileReadCorruptFile)
        }
        guard !vocabulary.isEmpty else { throw CocoaError(.fileReadCorruptFile) }
        return AsrModels(
            encoder: encoder, preprocessor: preprocessor, decoder: decoder, joint: joint,
            configuration: config, vocabulary: vocabulary, version: .v3)
    }
}
