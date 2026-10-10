// Test oracle: decode the audio track of an .mp4 the way QuickTime / Safari
// (AVFoundation) would — edit lists included — and write the result as a raw
// timeline of interleaved stereo f32 samples at 48 kHz, starting at
// presentation time 0 (gaps, e.g. an empty edit at the start, are zeros).
//
// usage: avf_decode <in.mp4> <out.f32> [audio-track-index]
import AVFoundation
import Foundation

let args = CommandLine.arguments
guard args.count == 3 || args.count == 4 else {
    FileHandle.standardError.write("usage: avf_decode <in.mp4> <out.f32> [audio-track-index]\n".data(using: .utf8)!)
    exit(2)
}
let trackIndex = args.count == 4 ? (Int(args[3]) ?? 0) : 0

let rate = 48_000.0
let asset = AVURLAsset(url: URL(fileURLWithPath: args[1]))
let audioTracks = asset.tracks(withMediaType: .audio)
guard trackIndex < audioTracks.count else {
    FileHandle.standardError.write("no audio track\n".data(using: .utf8)!)
    exit(3)
}
let track = audioTracks[trackIndex]

let reader = try AVAssetReader(asset: asset)
let output = AVAssetReaderTrackOutput(
    track: track,
    outputSettings: [
        AVFormatIDKey: kAudioFormatLinearPCM,
        AVSampleRateKey: rate,
        AVNumberOfChannelsKey: 2,
        AVLinearPCMBitDepthKey: 32,
        AVLinearPCMIsFloatKey: true,
        AVLinearPCMIsBigEndianKey: false,
        AVLinearPCMIsNonInterleaved: false,
    ])
reader.add(output)
guard reader.startReading() else {
    FileHandle.standardError.write("cannot read: \(String(describing: reader.error))\n".data(using: .utf8)!)
    exit(4)
}

let bytesPerFrame = 8  // 2 channels x f32
var timeline = Data()
while let sample = output.copyNextSampleBuffer() {
    guard let block = CMSampleBufferGetDataBuffer(sample) else { continue }
    var length = 0
    var pointer: UnsafeMutablePointer<Int8>? = nil
    guard CMBlockBufferGetDataPointer(block, atOffset: 0, lengthAtOffsetOut: nil,
                                      totalLengthOut: &length, dataPointerOut: &pointer) == noErr,
          let pointer = pointer else { continue }

    // Place the buffer at its presentation time, padding any gap with silence.
    let pts = CMSampleBufferGetPresentationTimeStamp(sample).seconds
    let startFrame = max(0, Int((pts * rate).rounded()))
    let have = timeline.count / bytesPerFrame
    if startFrame > have {
        timeline.append(Data(count: (startFrame - have) * bytesPerFrame))
    } else if startFrame < have {
        timeline.removeLast((have - startFrame) * bytesPerFrame)  // overlap: later buffer wins
    }
    timeline.append(Data(bytes: pointer, count: length))
}
if reader.status == .failed {
    FileHandle.standardError.write("read failed: \(String(describing: reader.error))\n".data(using: .utf8)!)
    exit(5)
}
try timeline.write(to: URL(fileURLWithPath: args[2]))
