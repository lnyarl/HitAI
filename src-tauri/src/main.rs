// Windows에서 릴리스 실행 시 콘솔 창이 뜨지 않게 한다.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    hitai_lib::run()
}
