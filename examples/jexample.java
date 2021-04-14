package com.example;

class Example {
    public static void main(String[] args) throws Exception {
	a();
    }

    private static void a() throws Exception {
	    b();
    }

    private static void b() throws Exception {
        System.out.println("Sleeping...");
        Thread.sleep(10000000);
    }
}

