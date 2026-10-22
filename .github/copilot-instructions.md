# Building and Testing

Use the build and test targets defined in "justfile" rather than directly using cargo or cross commands.
The justfile automatically does the right thing for each target.

Once you've completed any changes, you should make sure they build with `just build`. After major
changes, run local tests with `just ltest`.

# Etiquette

Do not create commits.
