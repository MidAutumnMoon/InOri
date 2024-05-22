alias t := test
[no-cd]
test capture="":
    cargo nextest run \
        {{ if capture == "" { "" } else { "--no-capture" } }}


alias tn := test-no-capture
[no-cd]
test-no-capture:
    just test 1
