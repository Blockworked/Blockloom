# HDR flicker repro (temporary, delete with the fix)

The user's "First Person" project, stripped for a GPU probe: the Enemy actor
and every `stop all` are gone so a run doesn't end itself, and no assets are
included (models and textures fall back to boxes and flat colors, which still
reproduces). Copy this folder somewhere writable before running the probe on it:

    cp -r wip/hdr-flicker /tmp/hdr-flicker && mkdir -p /tmp/hdr-flicker/assets
    BL_FAKE_HDR=/tmp/h.txt FP_PLAY=1 FP_DIR=/tmp/hdr-flicker FP_OUT=/tmp/x \
      cargo test --release -p blockloom-runtime --lib flicker_probe -- --ignored
    awk 'NR>200{t++; e=$1-q; if(e<0)e=-e; if(e>0.02*q)k++; q=$1} END{print t, k}' /tmp/h.txt

The second number is frames whose mean luminance jumped >2%: about 60-115 per
run before the fix, 0-1 once it's fixed.
