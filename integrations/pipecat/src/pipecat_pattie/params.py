"""PattieTransportParams: what the transport opens, and in what format."""

from pipecat.transports.base_transport import TransportParams


class PattieTransportParams(TransportParams):
    """Pipecat's transport parameters, plus the Devices'.

    Parameters:
        mic_name: The bot's microphone, as meeting apps list it.
        speaker_name: The bot's speaker, as meeting apps list it.
        device_rate: The Devices' sample rate: 44,100, 48,000 or 96,000.
        device_channels: The Devices' channels, 1 to 8. The pipeline's own
            format is converted to and from it.
        strict: Raise ``pattie.AudioLost`` when audio is lost, instead of
            logging a warning.
    """

    mic_name: str = "Agent Mic"
    speaker_name: str = "Agent Speaker"
    device_rate: int = 48_000
    device_channels: int = 1
    strict: bool = False
