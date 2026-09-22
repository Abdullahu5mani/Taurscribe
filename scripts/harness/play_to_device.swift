// Plays an audio file to ONE named output device (e.g. "BlackHole 2ch") without
// touching the system default output.
//
// The meeting E2E uses it to "speak as the local user": audio sent to BlackHole's
// output comes back out of BlackHole's input, which is set as the default mic.
//
// Usage: play_to_device "<device name>" <file.wav>

import AVFoundation
import CoreAudio
import Foundation

func fail(_ msg: String) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(1)
}

func deviceID(named name: String) -> AudioDeviceID? {
    var addr = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyDevices,
                                          mScope: kAudioObjectPropertyScopeGlobal,
                                          mElement: kAudioObjectPropertyElementMain)
    var size: UInt32 = 0
    guard AudioObjectGetPropertyDataSize(AudioObjectID(kAudioObjectSystemObject), &addr, 0, nil, &size) == noErr else { return nil }
    var ids = [AudioDeviceID](repeating: 0, count: Int(size) / MemoryLayout<AudioDeviceID>.size)
    guard AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &addr, 0, nil, &size, &ids) == noErr else { return nil }
    for id in ids {
        var nameAddr = AudioObjectPropertyAddress(mSelector: kAudioObjectPropertyName,
                                                  mScope: kAudioObjectPropertyScopeGlobal,
                                                  mElement: kAudioObjectPropertyElementMain)
        var cfName: Unmanaged<CFString>?
        var nameSize = UInt32(MemoryLayout<Unmanaged<CFString>?>.size)
        if AudioObjectGetPropertyData(id, &nameAddr, 0, nil, &nameSize, &cfName) == noErr,
           let n = cfName?.takeRetainedValue() as String?, n == name {
            return id
        }
    }
    return nil
}

let args = CommandLine.arguments
guard args.count == 3 else { fail("usage: play_to_device \"<device name>\" <file>") }
guard let device = deviceID(named: args[1]) else { fail("no audio device named '\(args[1])'") }
guard let file = try? AVAudioFile(forReading: URL(fileURLWithPath: args[2])) else { fail("cannot open \(args[2])") }

let engine = AVAudioEngine()
// Point this engine's output unit at the chosen device BEFORE building the graph;
// changing it afterwards leaves the engine rendering nowhere.
var dev = device
guard let outputUnit = engine.outputNode.audioUnit,
      AudioUnitSetProperty(outputUnit, kAudioOutputUnitProperty_CurrentDevice, kAudioUnitScope_Global, 0,
                           &dev, UInt32(MemoryLayout<AudioDeviceID>.size)) == noErr else {
    fail("could not route output to '\(args[1])'")
}
let player = AVAudioPlayerNode()
engine.attach(player)
engine.connect(player, to: engine.mainMixerNode, format: file.processingFormat)
engine.prepare()
do { try engine.start() } catch { fail("engine start failed: \(error)") }

let seconds = Double(file.length) / file.processingFormat.sampleRate
player.scheduleFile(file, at: nil, completionHandler: nil)
player.play()
// Wait for the file's length (plus drain) rather than a completion callback,
// which does not fire reliably for non-default devices.
Thread.sleep(forTimeInterval: seconds + 0.4)
player.stop()
engine.stop()
print(String(format: "played %.2fs to %@", seconds, args[1]))
