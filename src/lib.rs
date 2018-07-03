// Copyright (c) 2018  Brendan Molloy <brendan@bbqsrc.net>
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

#![feature(set_stdio)]
#![feature(fnbox)]

pub extern crate gherkin_rust as gherkin;
pub extern crate regex;
extern crate termcolor;
extern crate pathdiff;
extern crate textwrap;

use gherkin::{Step, StepType, Feature};
use regex::Regex;
use std::collections::HashMap;
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io;
use std::io::prelude::*;
use std::ops::Deref;
use std::panic;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::any::Any;
use std::io::Write;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};
use pathdiff::diff_paths;
use std::env;

pub trait World: Default {}

pub trait OutputVisitor : Default {
    fn visit_start(&mut self);
    fn visit_feature(&mut self, feature: &gherkin::Feature, path: &Path);
    fn visit_feature_end(&mut self, feature: &gherkin::Feature);
    fn visit_scenario(&mut self, scenario: &gherkin::Scenario);
    fn visit_scenario_end(&mut self, scenario: &gherkin::Scenario);
    fn visit_scenario_skipped(&mut self, scenario: &gherkin::Scenario);
    fn visit_step(&mut self, step: &gherkin::Step);
    fn visit_step_result(&mut self, step: &gherkin::Step, result: &TestResult);
    fn visit_finish(&mut self);
}

pub struct DefaultOutput {
    stdout: StandardStream,
    cur_feature: String,
    scenario_count: u32,
    scenario_skipped_count: u32,
    scenario_fail_count: u32,
    step_count: u32,
    skipped_count: u32,
    fail_count: u32
}

impl std::default::Default for DefaultOutput {
    fn default() -> DefaultOutput {
        DefaultOutput {
            stdout: StandardStream::stdout(ColorChoice::Always),
            cur_feature: "".to_string(),
            scenario_count: 0,
            scenario_skipped_count: 0,
            scenario_fail_count: 0,
            step_count: 0,
            skipped_count: 0,
            fail_count: 0
        }
    }
}

fn wrap_with_comment(s: &str, c: &str, indent: &str) -> String {
    let tw = textwrap::termwidth();
    let w = tw - indent.chars().count();
    let mut cs: Vec<String> = textwrap::wrap_iter(s, w)
        .map(|x| format!("{}{}", indent, &x.trim()))
        .collect();
    // Fit the comment onto the last line
    let comment_space = tw - c.chars().count() - 2;
    let last_count = cs.last().unwrap().chars().count();
    if last_count > comment_space {
        cs.push(format!("{: <1$}", "", comment_space))
    } else {
        cs.last_mut().unwrap().push_str(&format!("{: <1$}", "", comment_space - last_count));
    }
    cs.join("\n")
}

impl DefaultOutput {
    fn writeln(&mut self, s: &str, c: Color, bold: bool) {
        self.stdout.set_color(ColorSpec::new().set_fg(Some(c)).set_bold(bold)).unwrap();
        writeln!(&mut self.stdout, "{}", s).unwrap();
        self.stdout.set_color(ColorSpec::new().set_fg(None).set_bold(false)).unwrap();
    }

    fn writeln_cmt(&mut self, s: &str, cmt: &str, indent: &str, c: Color, bold: bool) {
        self.stdout.set_color(ColorSpec::new().set_fg(Some(c)).set_bold(bold)).unwrap();
        write!(&mut self.stdout, "{}", wrap_with_comment(s, cmt, indent)).unwrap();
        self.stdout.set_color(ColorSpec::new().set_fg(Some(Color::White)).set_bold(false)).unwrap();
        writeln!(&mut self.stdout, " {}", cmt).unwrap();
        self.stdout.set_color(ColorSpec::new().set_fg(None)).unwrap();
    }

    fn red(&mut self, s: &str) {
        self.writeln(s, Color::Red, false);
    }
    
    fn bold_white(&mut self, s: &str) {
        self.writeln(s, Color::Green, true);
    }

    fn bold_white_comment(&mut self, s: &str, c: &str, indent: &str) {
        self.writeln_cmt(s, c, indent, Color::White, true);
    }

    fn relpath(&self, target: &Path) -> std::path::PathBuf {
        diff_paths(&target, &env::current_dir().unwrap()).unwrap()
    }
}

impl OutputVisitor for DefaultOutput {
    fn visit_start(&mut self) {
        self.bold_white(&format!("[Cucumber v{}]\n", env!("CARGO_PKG_VERSION")))
    }

    fn visit_feature(&mut self, feature: &gherkin::Feature, path: &Path) {
        self.cur_feature = self.relpath(&path).to_string_lossy().to_string();
        let msg = &format!("Feature: {}", &feature.name);
        let cmt = &format!("{}:{}:{}", &self.cur_feature, feature.position.0, feature.position.1);
        self.bold_white_comment(msg, cmt, "");
        println!("");
    }
    
    fn visit_feature_end(&mut self, _feature: &gherkin::Feature) {}

    fn visit_scenario(&mut self, scenario: &gherkin::Scenario) {
        let cmt = &format!("{}:{}:{}", &self.cur_feature, scenario.position.0, scenario.position.1);
        self.bold_white_comment(&format!("Scenario: {}", &scenario.name), cmt, " ");
        self.scenario_count += 1;
    }

    fn visit_scenario_skipped(&mut self, _scenario: &gherkin::Scenario) {
        self.scenario_skipped_count += 1;
   }
    
    fn visit_scenario_end(&mut self, _scenario: &gherkin::Scenario) {
        println!("");
    }
    
    fn visit_step(&mut self, _step: &gherkin::Step) {
        self.step_count += 1;
    }
    
    fn visit_step_result(&mut self, step: &gherkin::Step, result: &TestResult) {
        let cmt = &format!("{}:{}:{}", &self.cur_feature, step.position.0, step.position.1);
        let msg = &format!("{}", &step.to_string());
        let indent = "  ";
        let ds = || {
            if let Some(ref docstring) = &step.docstring {
                println!("    \"\"\"\n    {}\n    \"\"\"", docstring);
            }
        };

        match result {
            TestResult::Pass => {
                self.writeln_cmt(&format!("✔ {}", msg), cmt, indent, Color::Green, false);
                ds();
            },
            TestResult::Fail(err_msg, loc) => {
                self.writeln_cmt(&format!("✘ {}", msg), cmt, indent, Color::Red, false);
                ds();
                self.writeln_cmt(&format!("{:-<1$}", "🚨 Step failed: ", textwrap::termwidth() - loc.chars().count() - 7), loc, "---- ", Color::Red, true);
                self.red(&textwrap::indent(&textwrap::fill(err_msg, textwrap::termwidth() - 4), "  ").trim_right());
                self.writeln(&format!("{:-<1$}", "", textwrap::termwidth()), Color::Red, true);
                self.fail_count += 1;
                self.scenario_fail_count += 1;
            },
            TestResult::MutexPoisoned => {
                self.writeln_cmt(&format!("- {}", msg), cmt, indent, Color::Cyan, false);
                ds();
                println!("      ⚡ Skipped due to previous error (poisoned)");
                self.fail_count += 1;
            },
            TestResult::Skipped => {
                self.writeln_cmt(&format!("- {}", msg), cmt, indent, Color::Cyan, false);
                ds();
                self.skipped_count += 1;
            }
            TestResult::Unimplemented => {
                self.writeln_cmt(&format!("- {}", msg), cmt, indent, Color::Cyan, false);
                ds();
                println!("      ⚡ Not yet implemented (skipped)");
                self.skipped_count += 1;
            }
        };
    }

    fn visit_finish(&mut self) {
        self.stdout.set_color(ColorSpec::new()
            .set_fg(Some(Color::Green))
            .set_bold(true)).unwrap();
            
        // Do scenario count
        let mut o = vec![];
        if self.fail_count > 0 {
            o.push(format!("{} failed", self.scenario_fail_count));
        }
        if self.skipped_count > 0 {
            o.push(format!("{} skipped", self.scenario_skipped_count));
        }
        write!(&mut self.stdout, "{} scenarios", &self.scenario_count).unwrap();
        if o.len() > 0 {
            write!(&mut self.stdout, " ({})", o.join(", ")).unwrap();
        }
        println!();

        // Do steps
        let mut o = vec![];
        if self.fail_count > 0 {
            o.push(format!("{} failed", self.fail_count));
        }
        if self.skipped_count > 0 {
            o.push(format!("{} skipped", self.skipped_count));
        }
        let passed_count = self.step_count - self.skipped_count - self.fail_count;
        o.push(format!("{} passed", passed_count));

        let msg = format!("{} steps ({})", &self.step_count, o.join(", "));
        writeln!(&mut self.stdout, "{}\n", &msg).unwrap();
        
        self.stdout.set_color(ColorSpec::new()
            .set_fg(None)
            .set_bold(false)).unwrap();
    }
}

#[derive(Debug, Clone)]
pub struct HashableRegex(pub Regex);

impl Hash for HashableRegex {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.as_str().hash(state);
    }
}

impl PartialEq for HashableRegex {
    fn eq(&self, other: &HashableRegex) -> bool {
        self.0.as_str() == other.0.as_str()
    }
}

impl Eq for HashableRegex {}

impl Deref for HashableRegex {
    type Target = Regex;

    fn deref(&self) -> &Regex {
        &self.0
    }
}

type TestFn<T> = fn(&mut T, &Step) -> ();
type TestRegexFn<T> = fn(&mut T, &[String], &Step) -> ();

pub struct TestCase<T: Default> {
    pub test: TestFn<T>
}

impl<T: Default> TestCase<T> {
    #[allow(dead_code)]
    pub fn new(test: TestFn<T>) -> TestCase<T> {
        TestCase {
            test: test
        }
    }
}

pub struct RegexTestCase<'a, T: 'a + Default> {
    pub test: TestRegexFn<T>,
    _marker: std::marker::PhantomData<&'a T>
}

impl<'a, T: Default> RegexTestCase<'a, T> {
    #[allow(dead_code)]
    pub fn new(test: TestRegexFn<T>) -> RegexTestCase<'a, T> {
        RegexTestCase {
            test: test,
            _marker: std::marker::PhantomData
        }
    }
}

pub struct Steps<'s, T: 's + Default> {
    pub given: HashMap<&'static str, TestCase<T>>,
    pub when: HashMap<&'static str, TestCase<T>>,
    pub then: HashMap<&'static str, TestCase<T>>,
    pub regex: RegexSteps<'s, T>
}

pub struct RegexSteps<'s, T: 's + Default> {
    pub given: HashMap<HashableRegex, RegexTestCase<'s, T>>,
    pub when: HashMap<HashableRegex, RegexTestCase<'s, T>>,
    pub then: HashMap<HashableRegex, RegexTestCase<'s, T>>,
}

pub enum TestCaseType<'a, T> where T: 'a, T: Default {
    Normal(&'a TestCase<T>),
    Regex(&'a RegexTestCase<'a, T>, Vec<String>)
}

pub enum TestResult {
    MutexPoisoned,
    Skipped,
    Unimplemented,
    Pass,
    Fail(String, String)
}

struct Sink(Arc<Mutex<Vec<u8>>>);
impl Write for Sink {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        Write::write(&mut *self.0.lock().unwrap(), data)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct CapturedIo<T> {
    stdout: Vec<u8>,
    result: Result<T, Box<dyn Any + Send>>
}

fn capture_io<T, F: FnOnce() -> T>(callback: F) -> CapturedIo<T> {
    let data = Arc::new(Mutex::new(Vec::new()));
    let data2 = data.clone();

    let old_io = (
        io::set_print(Some(Box::new(Sink(data2.clone())))),
        io::set_panic(Some(Box::new(Sink(data2))))
    );

    let result = panic::catch_unwind(
        panic::AssertUnwindSafe(callback)
    );

    let captured_io = CapturedIo {
        stdout: data.lock().unwrap().to_vec(),
        result: result
    };

    io::set_print(old_io.0);
    io::set_panic(old_io.1);

    captured_io
}


impl<'s, T: Default> Steps<'s, T> {
    #[allow(dead_code)]
    pub fn new() -> Steps<'s, T> {
        let regex_tests = RegexSteps {
            given: HashMap::new(),
            when: HashMap::new(),
            then: HashMap::new()
        };

        let tests = Steps {
            given: HashMap::new(),
            when: HashMap::new(),
            then: HashMap::new(),
            regex: regex_tests
        };

        tests
    }

    fn test_bag_for<'a>(&self, ty: StepType) -> &HashMap<&'static str, TestCase<T>> {
        match ty {
            StepType::Given => &self.given,
            StepType::When => &self.when,
            StepType::Then => &self.then
        }
    }

    fn regex_bag_for<'a>(&'a self, ty: StepType) -> &HashMap<HashableRegex, RegexTestCase<'a, T>> {
        match ty {
            StepType::Given => &self.regex.given,
            StepType::When => &self.regex.when,
            StepType::Then => &self.regex.then
        }
    }

    fn test_type(&'s self, step: &Step) -> Option<TestCaseType<'s, T>> {
        let test_bag = self.test_bag_for(step.ty);

        match test_bag.get(&*step.value) {
            Some(v) => Some(TestCaseType::Normal(v)),
            None => {
                let regex_bag = self.regex_bag_for(step.ty);

                let result = regex_bag.iter()
                    .find(|(regex, _)| regex.is_match(&step.value));

                match result {
                    Some((regex, tc)) => {
                        let matches = regex.0.captures(&step.value).unwrap();
                        let matches: Vec<String> = matches.iter().map(|x| x.unwrap().as_str().to_string()).collect();
                        Some(TestCaseType::Regex(tc, matches))
                    },
                    None => {
                        None
                    }
                }
            }
        }
    }

    fn run_test_inner<'a>(
        &'s self,
        world: &mut T,
        test_type: TestCaseType<'s, T>,
        step: &'a gherkin::Step
    ) {
        match test_type {
            TestCaseType::Normal(t) => (t.test)(world, &step),
            TestCaseType::Regex(t, ref c) => (t.test)(world, c, &step)
        };
    }

    fn run_test<'a>(&'s self, world: &mut T, test_type: TestCaseType<'s, T>, step: &'a Step, last_panic: Arc<Mutex<Option<String>>>) -> TestResult {
        let last_panic_hook = last_panic.clone();
        panic::set_hook(Box::new(move |info| {
            let mut state = last_panic.lock().expect("last_panic unpoisoned");
            *state = info.location().map(|x| format!("{}:{}:{}", x.file(), x.line(), x.column()));
        }));


        let captured_io = capture_io(move || {
            self.run_test_inner(world, test_type, &step)
        });

        let _ = panic::take_hook();
        
        match captured_io.result {
            Ok(_) => TestResult::Pass,
            Err(any) => {
                let mut state = last_panic_hook.lock().expect("unpoisoned");
                let loc = match &*state {
                    Some(v) => &v,
                    None => "unknown"
                };

                let s = {
                    if let Some(s) = any.downcast_ref::<String>() {
                        s.as_str()
                    } else if let Some(s) = any.downcast_ref::<&str>() {
                        *s
                    } else {
                        ""
                    }
                };

                if s == "not yet implemented" {
                    TestResult::Unimplemented
                } else {
                    let panic_str = if &captured_io.stdout.len() > &0usize {
                        String::from_utf8_lossy(&captured_io.stdout).to_string()
                    } else {
                        format!("Panicked with: {}", s)
                    };
                    TestResult::Fail(panic_str, loc.to_owned())
                }
            }
        }
    }

    fn run_scenario<'a>(
        &'s self,
        feature: &'a gherkin::Feature,
        scenario: &'a gherkin::Scenario,
        last_panic: Arc<Mutex<Option<String>>>,
        output: &mut impl OutputVisitor
    ) {
        output.visit_scenario(&scenario);

        let captured_io = capture_io(|| T::default());
        let mut world = match captured_io.result {
            Ok(v) => v,
            Err(e) => {
                if &captured_io.stdout.len() > &0usize {
                    let msg = String::from_utf8_lossy(&captured_io.stdout).to_string();
                    panic!(msg);
                } else {
                    panic!(e);
                }
            }
        };
        
        let mut steps: Vec<&'a Step> = vec![];
        if let Some(ref bg) = &feature.background {
            for s in &bg.steps {
                steps.push(&s);
            }
        }

        for s in &scenario.steps {
            steps.push(&s);
        }

        let mut is_skipping = false;

        for step in steps.iter() {
            output.visit_step(&step);

            let test_type = match self.test_type(&step) {
                Some(v) => v,
                None => {
                    output.visit_step_result(&step, &TestResult::Unimplemented);
                    if !is_skipping {
                        is_skipping = true;
                        output.visit_scenario_skipped(&scenario);
                    }
                    continue;
                }
            };

            if is_skipping {
                output.visit_step_result(&step, &TestResult::Skipped);
            } else {
                let result = self.run_test(&mut world, test_type, &step, last_panic.clone());
                output.visit_step_result(&step, &result);
                match result {
                    TestResult::Pass => {}
                    _ => {
                        is_skipping = true;
                        output.visit_scenario_skipped(&scenario);
                    }
                };
            }
        }

        output.visit_scenario_end(&scenario);
    }
    
    pub fn run<'a>(&'s self, feature_path: &Path, output: &mut impl OutputVisitor) {
        output.visit_start();
        
        let feature_path = fs::read_dir(feature_path).expect("feature path to exist");
        let last_panic: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        for entry in feature_path {
            let path = entry.unwrap().path();
            let mut file = File::open(&path).expect("file to open");
            let mut buffer = String::new();
            file.read_to_string(&mut buffer).unwrap();
            
            let feature = Feature::from(&*buffer);
            output.visit_feature(&feature, &path);

            for scenario in (&feature.scenarios).iter() {
                self.run_scenario(&feature, &scenario, last_panic.clone(), output);
            }

            output.visit_feature_end(&feature);
        }
        
        output.visit_finish();
    }
}

#[macro_export]
macro_rules! cucumber {
    (
        features: $featurepath:tt;
        world: $worldtype:path;
        steps: $vec:expr;
        before: $beforefn:expr
    ) => {
        cucumber!(@finish; $featurepath; $worldtype; $vec; Some(Box::new($beforefn)));
    };

    (
        features: $featurepath:tt;
        world: $worldtype:path;
        steps: $vec:expr
    ) => {
        cucumber!(@finish; $featurepath; $worldtype; $vec; None);
    };

    (
        @finish; $featurepath:tt; $worldtype:path; $vec:expr; $beforefn:expr
    ) => {
        #[allow(unused_imports)]
        fn main() {
            use std::path::Path;
            use std::process;
            use std::boxed::FnBox;
            use $crate::{Steps, World, DefaultOutput};

            let path = match Path::new($featurepath).canonicalize() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("{}", e);
                    eprintln!("There was an error parsing \"{}\"; aborting.", $featurepath);
                    process::exit(1);
                }
            };

            if !&path.exists() {
                eprintln!("Path {:?} does not exist; aborting.", &path);
                process::exit(1);
            }

            let tests = {
                let step_groups: Vec<Steps<$worldtype>> = $vec.iter().map(|f| f()).collect();
                let mut combined_steps = Steps::new();

                for step_group in step_groups.into_iter() {
                    combined_steps.given.extend(step_group.given);
                    combined_steps.when.extend(step_group.when);
                    combined_steps.then.extend(step_group.then);

                    combined_steps.regex.given.extend(step_group.regex.given);
                    combined_steps.regex.when.extend(step_group.regex.when);
                    combined_steps.regex.then.extend(step_group.regex.then);
                }

                combined_steps
            };
            
            let mut output = DefaultOutput::default();

            let before_fn: Option<Box<FnBox() -> ()>> = $beforefn;

            match before_fn {
                Some(f) => f(),
                None => {}
            };

            tests.run(&path, &mut output);
        }
    }
}

#[macro_export]
macro_rules! steps {
    (
        @gather_steps, $tests:tt,
        $ty:ident regex $name:tt $body:expr;
    ) => {
        $tests.regex.$ty.insert(
            HashableRegex(Regex::new($name).expect(&format!("{} is a valid regex", $name))),
                RegexTestCase::new($body));
    };

    (
        @gather_steps, $tests:tt,
        $ty:ident regex $name:tt $body:expr; $( $items:tt )*
    ) => {
        $tests.regex.$ty.insert(
            HashableRegex(Regex::new($name).expect(&format!("{} is a valid regex", $name))),
                RegexTestCase::new($body));

        steps!(@gather_steps, $tests, $( $items )*);
    };

    (
        @gather_steps, $tests:tt,
        $ty:ident $name:tt $body:expr;
    ) => {
        $tests.$ty.insert($name, TestCase::new($body));
    };

    (
        @gather_steps, $tests:tt,
        $ty:ident $name:tt $body:expr; $( $items:tt )*
    ) => {
        $tests.$ty.insert($name, TestCase::new($body));

        steps!(@gather_steps, $tests, $( $items )*);
    };

    (
        world: $worldtype:path;
        $( $items:tt )*
    ) => {
        #[allow(unused_imports)]
        pub fn steps<'a>() -> $crate::Steps<'a, $worldtype> {
            use std::path::Path;
            use std::process;
            use $crate::regex::Regex;
            use $crate::{Steps, TestCase, RegexTestCase, HashableRegex};

            let mut tests: Steps<'a, $worldtype> = Steps::new();
            steps!(@gather_steps, tests, $( $items )*);
            tests
        }
    };
}


#[cfg(test)]
mod tests {
    use std::default::Default;

    pub struct World {
        pub thing: bool
    }

    impl ::World for World {}

    impl Default for World {
        fn default() -> World {
            World {
                thing: false
            }
        }
    }
}

#[cfg(test)]
mod tests1 {
    steps! {
        world: ::tests::World;
        when regex "^test (.*) regex$" |_world, matches, _step| {
            println!("{}", matches[1]);
        };

        given "a thing" |_world, _step| {
            assert!(true);
        };

        when "another thing" |_world, _step| {
            assert!(false);
        };

        when "something goes right" |_world, _step| { 
            assert!(true);
        };

        then "another thing" |_world, _step| {
            assert!(true)
        };

        when "nothing" |world, step| {
            panic!("oh shit");
        };
    }
}

#[cfg(test)]
cucumber! {
    features: "./features";
    world: tests::World;
    steps: &[
        tests1::steps
    ]
}