# Blobscan indexer <a href="#"><img align="right" src=".github/assets/logo.svg" height="80px" /></a>

The indexer for the [Blobscan](https://github.com/Blobscan/blobscan) explorer implemented in Rust.

# Installation and usage

Check out our [documentation website](https://docs.blobscan.com/docs/indexer).

```
./blob-indexer --help
Blobscan's indexer for blob transactions (EIP-4844).

Usage: blob-indexer [OPTIONS]

Options:
  -f, --from-slot <FROM_SLOT>
          Slot to start indexing from
  -t, --to-slot <TO_SLOT>
          Slot to stop indexing at
  -n, --num-threads <NUM_THREADS>
          Number of threads used for parallel indexing
  -s, --slots-per-save <SLOTS_PER_SAVE>
          Amount of slots to be processed before saving latest slot in the database
  -c, --disable-sync-checkpoint-save
          Disable slot checkpoint saving when syncing
  -d, --disable-sync-historical
          Disable backfill indexing thread
  -h, --help
          Print help
  -V, --version
          Print version
```

# Sponsors

We extend our gratitude to each one of them. Thank you 🙏

<p>
  <a href="https://ethereum.foundation">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://esp.ethereum.foundation/_next/static/media/esp-logo.96fc01cc.svg"/>
      <img alt="paradigm logo" src="https://esp.ethereum.foundation/_next/static/media/esp-logo.96fc01cc.svg" width="auto" height="50"/>
    </picture>
  </a>
  &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;
  <a href="https://www.optimism.io">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/ethereum-optimism/brand-kit/main/assets/svg/Profile-Logo.svg"/>
      <img alt="optimism" src="https://raw.githubusercontent.com/ethereum-optimism/brand-kit/main/assets/svg/Profile-Logo.svg" width="auto" height="50"/>
    </picture>
  </a>
  &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;
  <a href="https://scroll.io">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://scroll.io/static/media/Scroll_InvertedLogo.ea3b717f2a3ae7275378c2d43550dd38.svg"/>
      <img alt="context logo" src="https://scroll.io/static/media/Scroll_FullLogo.07032ebd8a84b79128eb669f2822bc5e.svg" width="auto" height="50"/>
    </picture>
  </a>
  &nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;&nbsp;
  <a href="https://www.ethswarm.org">
    <picture>
      <source media="(prefers-color-scheme: dark)" srcset="https://docs.ethswarm.org/img/logo.svg"/>
      <img alt="context logo" src="https://docs.ethswarm.org/img/logo.svg" width="auto" height="50"/>
    </picture>
  </a>
</p>


#

[Join us on Discord!](https://discordapp.com/invite/fmqrqhkjHY/)
