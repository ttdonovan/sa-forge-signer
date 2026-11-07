How to read this book: choose chapters from the index below by their description and size, and read only those. In a local build (`target/docs/html/`) each linked file also sits beside this one and matches the checkout; the site follows `main`, which can be ahead of the release in use (`sa-forge-signer --version`).

The rule everything else depends on: the agent must not be able to read the signer's key directory or config. If it can, the signer's checks are advice, not a control; tell the human (see Choosing a setup, "Isolation").
