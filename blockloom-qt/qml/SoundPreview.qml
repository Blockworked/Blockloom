import QtQuick
import QtMultimedia

// Plays one sound file in place, apart from the game runtime. Loaded on its
// own so a Qt without multimedia just goes without previews.
Item {
    id: root
    property url source: ""
    readonly property bool playing: player.playbackState === MediaPlayer.PlayingState
    function play(url) { player.stop(); player.source = url; player.play(); }
    function stop() { player.stop(); }
    MediaPlayer { id: player; audioOutput: AudioOutput {} }
}
