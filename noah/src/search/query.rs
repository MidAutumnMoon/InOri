use elasticsearch_dsl::{Operator, Query, Search, TextQueryType};

const TYPE_OPTION: &str = "option";
const TYPE_SERVICE: &str = "service";
const OPTION_TYPES: &[&str] = &[TYPE_OPTION, TYPE_SERVICE];

pub fn packages(query: &str, limit: u64) -> Search {
    Search::new().from(0).size(limit).query(
        Query::bool().filter(Query::term("type", "package")).must(
            Query::dis_max()
                .tie_breaker(0.7)
                .query(
                    Query::multi_match(
                        [
                            "package_attr_name^9",
                            "package_attr_name.*^5.3999999999999995",
                            "package_programs^9",
                            "package_programs.*^5.3999999999999995",
                            "package_pname^6",
                            "package_pname.*^3.5999999999999996",
                            "package_description^1.3",
                            "package_description.*^0.78",
                            "package_longDescription^1",
                            "package_longDescription.*^0.6",
                            "flake_name^0.5",
                            "flake_name.*^0.3",
                        ],
                        query.to_owned(),
                    )
                    .r#type(TextQueryType::CrossFields)
                    .analyzer("whitespace")
                    .auto_generate_synonyms_phrase_query(false)
                    .operator(Operator::And),
                )
                .query(
                    Query::wildcard(
                        "package_attr_name",
                        format!("*{query}*"),
                    )
                    .case_insensitive(true),
                ),
        ),
    )
}

pub fn options(query: &str, limit: u64) -> Search {
    Search::new().from(0).size(limit).query(
        Query::bool()
            .filter(Query::terms("type", OPTION_TYPES))
            .must(
                Query::dis_max()
                    .tie_breaker(0.7)
                    .query(
                        Query::multi_match(
                            [
                                "option_name^6",
                                "option_name.*^3.6",
                                "option_name_query^6",
                                "option_name_query.*^3.6",
                                "option_description^1",
                                "option_description.*^0.6",
                                "flake_name^0.5",
                                "flake_name.*^0.3",
                                "service_package^3",
                                "service_package.*^1.8",
                                "service_packages^3",
                                "service_packages.*^1.8",
                            ],
                            query.to_owned(),
                        )
                        .r#type(TextQueryType::CrossFields)
                        .analyzer("whitespace")
                        .auto_generate_synonyms_phrase_query(false)
                        .operator(Operator::And),
                    )
                    .query(
                        Query::wildcard(
                            "option_name",
                            format!("*{query}*"),
                        )
                        .case_insensitive(true),
                    ),
            ),
    )
}
